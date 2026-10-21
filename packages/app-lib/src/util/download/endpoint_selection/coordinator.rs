use super::candidates::{collect_candidates, eligible_target};
use super::model::{
    EndpointCandidate, EndpointLease, NetworkContext, PoolKey, ProbeFailure,
    ProbeTarget, Protocol,
};
use super::policy::{self, ProbeBudget};
use super::probe::{ProbeObservation, ProbeSession};
use crate::util::download::{modrinth_cdn, native, native_breaker};
use crate::util::fetch::{
    DownloadClients, DownloadRequest, DownloadRoute, ProxyPolicy,
};
use crate::util::proxy::ProxyMode;
use futures::{StreamExt, stream};
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{Semaphore, watch};
use tokio_util::sync::CancellationToken;

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
static PROBE_ROUNDS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(1)));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectionMode {
    Off,
    Observe,
    Auto,
}

impl SelectionMode {
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("observe") => Self::Observe,
            Some("auto") => Self::Auto,
            // Enable by default only after the full recovery regression gate.
            None | Some("off") => Self::Off,
            Some(_) => Self::Off,
        }
    }

    fn configured() -> Self {
        static MODE: LazyLock<SelectionMode> = LazyLock::new(|| {
            let value = std::env::var("AXOLOTL_MODRINTH_IP_SELECTION").ok();
            if value
                .as_deref()
                .is_some_and(|s| !matches!(s, "off" | "observe" | "auto"))
            {
                tracing::warn!("Invalid Modrinth IP selection mode; using off");
            }
            SelectionMode::parse(value.as_deref())
        });
        *MODE
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RoundStatus {
    Running,
    Selected,
    NoGain,
    Cancelled,
    Stale,
    BudgetExhausted,
}

#[derive(Clone)]
struct Selection {
    protocol: Protocol,
    preferred: EndpointCandidate,
    backups: Vec<EndpointCandidate>,
    speed: u64,
    expires_at: Instant,
}

struct ActiveRound {
    id: u64,
    epoch: u64,
    generation: u64,
    cancellation: CancellationToken,
    status: watch::Sender<RoundStatus>,
}

struct State {
    #[cfg(test)]
    endpoint_port: u16,
    closed: bool,
    epoch: u64,
    generation: u64,
    next_round: u64,
    next_registration: u64,
    registrations: HashMap<u64, CancellationToken>,
    round: Option<ActiveRound>,
    selected: Option<Selection>,
    previous: Option<Selection>,
    failures: HashMap<IpAddr, u8>,
    cooldown_until: Option<Instant>,
    last_activity: Instant,
    hourly_reservations: VecDeque<(Instant, u64)>,
}

pub(crate) struct EndpointSelector {
    mode: SelectionMode,
    config_scope: Arc<str>,
    state: Mutex<State>,
    probe_bytes: Arc<AtomicU64>,
    pub(super) pinned_clients: super::clients::PinnedClients,
}

/// Removing one registration cannot cancel a probe needed by another file.
pub(crate) struct DownloadRegistration {
    selector: Weak<EndpointSelector>,
    id: u64,
}

impl Drop for DownloadRegistration {
    fn drop(&mut self) {
        if let Some(selector) = self.selector.upgrade() {
            let mut state = selector.state.lock();
            state.registrations.remove(&self.id);
            state.registrations.retain(|_, token| !token.is_cancelled());
            if state.registrations.is_empty()
                && let Some(round) = &state.round
            {
                round.cancellation.cancel();
            }
        }
    }
}

/// Created before spawning: even an unpolled or panicking worker clears its
/// slot and leaves a retained terminal result for every watch subscriber.
struct RoundGuard {
    selector: Weak<EndpointSelector>,
    id: u64,
    status: watch::Sender<RoundStatus>,
    finished: bool,
}

impl RoundGuard {
    fn finish(mut self, status: RoundStatus) {
        self.finished = true;
        self.status.send_replace(status);
        self.clear();
    }

    fn clear(&self) {
        if let Some(selector) = self.selector.upgrade() {
            let mut state = selector.state.lock();
            if state
                .round
                .as_ref()
                .is_some_and(|round| round.id == self.id)
            {
                state.round = None;
                state.cooldown_until = Some(Instant::now() + policy::COOLDOWN);
            }
        }
    }
}

impl Drop for RoundGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.status.send_replace(RoundStatus::Cancelled);
            self.clear();
        }
    }
}

struct RoundStart {
    guard: RoundGuard,
    cancellation: CancellationToken,
    epoch: u64,
    generation: u64,
    allowance: u64,
}

impl EndpointSelector {
    pub(crate) fn new(scope: impl Into<Arc<str>>) -> Arc<Self> {
        Self::with_mode(scope, SelectionMode::configured())
    }

    pub(super) fn with_mode(
        scope: impl Into<Arc<str>>,
        mode: SelectionMode,
    ) -> Arc<Self> {
        Arc::new(Self {
            pinned_clients: super::clients::PinnedClients::default(),
            mode,
            config_scope: scope.into(),
            probe_bytes: Arc::new(AtomicU64::new(0)),
            state: Mutex::new(State {
                #[cfg(test)]
                endpoint_port: 443,
                closed: false,
                epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
                generation: 0,
                next_round: 0,
                next_registration: 0,
                registrations: HashMap::new(),
                round: None,
                selected: None,
                previous: None,
                failures: HashMap::new(),
                cooldown_until: None,
                last_activity: Instant::now(),
                hourly_reservations: VecDeque::new(),
            }),
        })
    }

    pub(crate) fn mode(&self) -> SelectionMode {
        self.mode
    }

    pub(crate) fn eligible_network(
        clients: &DownloadClients,
        proxy: ProxyPolicy,
    ) -> bool {
        !clients.ignore_ssl_errors
            && (proxy == ProxyPolicy::Direct
                || clients.proxy.mode == ProxyMode::None
                || clients.proxy.mode == ProxyMode::System
                    && !native::system_proxy_configured())
    }

    pub(crate) fn register_download(
        self: &Arc<Self>,
        request: &DownloadRequest,
    ) -> Option<DownloadRegistration> {
        if self.mode == SelectionMode::Off {
            return None;
        }
        let url =
            url::Url::parse(&modrinth_cdn::normalize(&request.url)).ok()?;
        if !eligible_target(&url, &request.integrity)
            || request.header.is_some()
        {
            return None;
        }
        self.register(request.cancellation.clone().unwrap_or_default())
    }

    fn register(
        self: &Arc<Self>,
        cancellation: CancellationToken,
    ) -> Option<DownloadRegistration> {
        let mut state = self.state.lock();
        if state.closed || cancellation.is_cancelled() {
            return None;
        }
        Self::expire(&mut state, Instant::now());
        state.next_registration += 1;
        let id = state.next_registration;
        state.registrations.insert(id, cancellation);
        Some(DownloadRegistration {
            selector: Arc::downgrade(self),
            id,
        })
    }

    fn expire(state: &mut State, now: Instant) {
        if now.saturating_duration_since(state.last_activity)
            >= policy::SELECTION_LIFETIME
        {
            state.epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
            state.generation += 1;
            state.selected = None;
            state.previous = None;
            state.failures.clear();
            if let Some(round) = &state.round {
                round.cancellation.cancel();
            }
        }
        state.last_activity = now;
        if state
            .selected
            .as_ref()
            .is_some_and(|selection| selection.expires_at <= now)
        {
            state.selected = None;
            state.previous = None;
            state.generation += 1;
        }
    }

    pub(crate) fn lease(
        &self,
        url: &url::Url,
        protocol: Protocol,
    ) -> Option<EndpointLease> {
        if self.mode != SelectionMode::Auto
            || !modrinth_cdn::is_current_url(url)
        {
            return None;
        }
        let mut state = self.state.lock();
        if state.closed {
            return None;
        }
        let now = Instant::now();
        Self::expire(&mut state, now);
        let selection = state
            .selected
            .as_ref()
            .filter(|s| s.protocol == protocol && s.expires_at > now)?;
        Some(EndpointLease {
            pool: PoolKey {
                authority: Arc::from("cdn.modrinth.com:443"),
                network: NetworkContext {
                    config_scope: self.config_scope.clone(),
                    epoch: state.epoch,
                },
                generation: state.generation,
                endpoint: SocketAddr::new(selection.preferred.ip, {
                    #[cfg(test)]
                    {
                        state.endpoint_port
                    }
                    #[cfg(not(test))]
                    {
                        443
                    }
                }),
                protocol,
            },
            source: selection.preferred.source.clone(),
            expires_at: selection.expires_at,
        })
    }

    pub(crate) fn is_current(&self, lease: &EndpointLease) -> bool {
        self.lease(
            &url::Url::parse("https://cdn.modrinth.com/").unwrap(),
            lease.pool.protocol,
        )
        .is_some_and(|current| current.pool == lease.pool)
    }

    pub(crate) fn close(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        state.selected = None;
        state.previous = None;
        state.generation += 1;
        if let Some(round) = &state.round {
            round.cancellation.cancel();
        }
        drop(state);
        self.pinned_clients.clear();
    }

    pub(crate) fn record_failure(
        &self,
        lease: &EndpointLease,
        invalid_certificate: bool,
    ) {
        let mut state = self.state.lock();
        if state.closed
            || state.epoch != lease.pool.network.epoch
            || state.generation != lease.pool.generation
        {
            return;
        }
        let failures =
            state.failures.entry(lease.pool.endpoint.ip()).or_default();
        *failures = failures.saturating_add(1);
        if !invalid_certificate && *failures < 2 {
            return;
        }
        let now = Instant::now();
        if let Some(mut selection) = state.selected.take() {
            selection.backups.retain(|candidate| {
                candidate.ip != lease.pool.endpoint.ip()
                    && candidate.expires_at > now
            });
            if !selection.backups.is_empty() {
                selection.preferred = selection.backups.remove(0);
                selection.expires_at =
                    selection.expires_at.min(selection.preferred.expires_at);
                state.selected = Some(selection);
            }
        }
        state.generation += 1;
    }

    fn begin(self: &Arc<Self>, now: Instant) -> Option<RoundStart> {
        if self.mode != SelectionMode::Auto {
            return None;
        }
        let mut state = self.state.lock();
        Self::expire(&mut state, now);
        state.registrations.retain(|_, token| !token.is_cancelled());
        if state.closed
            || state.registrations.is_empty()
            || state.round.is_some()
            || state.cooldown_until.is_some_and(|until| until > now)
        {
            return None;
        }
        while state.hourly_reservations.front().is_some_and(|(at, _)| {
            now.saturating_duration_since(*at) >= Duration::from_secs(3600)
        }) {
            state.hourly_reservations.pop_front();
        }
        let allowance = policy::HOURLY_BYTES
            .saturating_sub(
                state
                    .hourly_reservations
                    .iter()
                    .map(|(_, bytes)| bytes)
                    .sum::<u64>(),
            )
            .min(policy::ROUND_BYTES);
        if allowance < policy::VALIDATION_BYTES {
            return None;
        }
        // Reserve the complete round up front. Cancellations conservatively
        // keep their allowance: reconnecting cannot reset the hourly cap.
        state.hourly_reservations.push_back((now, allowance));
        state.next_round += 1;
        let id = state.next_round;
        let (status, _) = watch::channel(RoundStatus::Running);
        let cancellation = CancellationToken::new();
        let epoch = state.epoch;
        let generation = state.generation;
        state.round = Some(ActiveRound {
            id,
            epoch,
            generation,
            cancellation: cancellation.clone(),
            status: status.clone(),
        });
        Some(RoundStart {
            guard: RoundGuard {
                selector: Arc::downgrade(self),
                id,
                status,
                finished: false,
            },
            cancellation,
            epoch,
            generation,
            allowance,
        })
    }

    pub(crate) fn on_slow(
        self: &Arc<Self>,
        request: &DownloadRequest,
        route: &DownloadRoute,
        clients: &DownloadClients,
        protocol: Protocol,
        incumbent: Option<IpAddr>,
        remaining: u64,
        pressure: super::super::local_resources::Pressure,
    ) {
        let Ok(url) = url::Url::parse(&route.url) else {
            return;
        };
        if !Self::eligible_network(clients, route.proxy)
            || !eligible_target(&url, &request.integrity)
            || request.header.is_some()
            || native_breaker::is_open(route)
            || pressure.blocks_expansion()
        {
            return;
        }
        let Some(start) = self.begin(Instant::now()) else {
            return;
        };
        let selector = self.clone();
        let route = route.clone();
        let clients = clients.clone();
        let target = ProbeTarget {
            url,
            size: request.integrity.size.unwrap_or(0),
            protocol,
        };
        tokio::spawn(async move {
            let RoundStart {
                guard,
                cancellation,
                epoch,
                generation,
                allowance,
            } = start;
            let operation = async {
                let Ok(_global) = PROBE_ROUNDS.clone().try_acquire_owned()
                else {
                    return RoundStatus::BudgetExhausted;
                };
                if !matches!(
                    crate::util::network::is_network_metered().await,
                    Ok(false)
                ) {
                    return RoundStatus::NoGain;
                }
                selector
                    .run_round(
                        &target,
                        &route,
                        &clients,
                        incumbent,
                        remaining,
                        epoch,
                        generation,
                        allowance,
                        &cancellation,
                    )
                    .await
            };
            let status = tokio::select! {
                biased;
                _ = cancellation.cancelled() => RoundStatus::Cancelled,
                _ = selector.wait_for_no_observers() => RoundStatus::Cancelled,
                result = tokio::time::timeout(policy::ROUND_TIMEOUT,
                    super::super::proxy_context::with_snapshot(clients.clone(), operation)) => result.unwrap_or(RoundStatus::NoGain),
            };
            guard.finish(status);
        });
    }

    async fn wait_for_no_observers(&self) {
        loop {
            {
                let mut state = self.state.lock();
                state.registrations.retain(|_, token| !token.is_cancelled());
                if state.closed || state.registrations.is_empty() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    pub(crate) fn is_probing(&self) -> bool {
        self.state
            .lock()
            .round
            .as_ref()
            .is_some_and(|round| !round.cancellation.is_cancelled())
    }

    async fn run_round(
        &self,
        target: &ProbeTarget,
        route: &DownloadRoute,
        clients: &DownloadClients,
        incumbent: Option<IpAddr>,
        remaining: u64,
        epoch: u64,
        generation: u64,
        allowance: u64,
        cancellation: &CancellationToken,
    ) -> RoundStatus {
        let started = Instant::now();
        let deadline = started + policy::ROUND_TIMEOUT;
        let resolver = if route.proxy == ProxyPolicy::Direct {
            &clients.direct_dns
        } else {
            &clients.system_dns
        };
        let Ok(answers) = resolver
            .candidate_snapshot(
                modrinth_cdn::HOST,
                cancellation,
                deadline.min(started + Duration::from_secs(5)),
            )
            .await
        else {
            return RoundStatus::NoGain;
        };
        let verified = {
            let state = self.state.lock();
            state
                .selected
                .as_ref()
                .map(|s| {
                    std::iter::once(s.preferred.clone())
                        .chain(s.backups.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let candidates = collect_candidates(
            &answers,
            &verified,
            &[],
            true,
            Instant::now(),
            SystemTime::now(),
        );
        let budget = Mutex::new(ProbeBudget::new(started, allowance));
        tracing::debug!(
            authority = modrinth_cdn::HOST,
            epoch,
            generation,
            candidates = candidates.len(),
            "Starting Modrinth endpoint trial"
        );
        let mut pending =
            stream::iter(candidates.into_iter().map(|candidate| async {
                let session = ProbeSession::open(
                    candidate.clone(),
                    target.clone(),
                    route,
                    clients,
                    self.probe_bytes.clone(),
                )?;
                let observation = session
                    .validate_endpoint(&budget, cancellation, deadline)
                    .await?;
                Ok::<_, ProbeFailure>((candidate, observation))
            }))
            .buffer_unordered(policy::VALIDATION_CONCURRENCY);
        let mut validated = Vec::new();
        while let Some(result) = pending.next().await {
            match result {
                Ok(result) => validated.push(result),
                Err(ProbeFailure::ServerCooldown) => {
                    native_breaker::record_response_status_failure(
                        route,
                        429,
                        Some(policy::COOLDOWN),
                    );
                    return RoundStatus::NoGain;
                }
                Err(ProbeFailure::TargetMissing | ProbeFailure::Cancelled) => {
                    return RoundStatus::NoGain;
                }
                Err(_) => {}
            }
        }
        drop(pending);
        // Sample the incumbent and distinct seed/family paths before filling
        // remaining slots with the ordinary anycast pool.
        validated.sort_by_key(|(candidate, _)| {
            (
                Some(candidate.ip) != incumbent,
                !matches!(
                    candidate.source,
                    super::model::CandidateSource::FastlyRule(_)
                ),
                candidate.ip.is_ipv4(),
                candidate.ip,
            )
        });
        validated.truncate(policy::MAX_MEASURED_CANDIDATES);
        let mut samples = Vec::new();
        for _ in 0..2 {
            for (candidate, _) in &validated {
                if Instant::now() >= deadline
                    || budget.lock().remaining(Instant::now()) == 0
                {
                    break;
                }
                let Ok(session) = ProbeSession::open(
                    candidate.clone(),
                    target.clone(),
                    route,
                    clients,
                    self.probe_bytes.clone(),
                ) else {
                    continue;
                };
                match session
                    .sample_endpoint(&budget, cancellation, deadline)
                    .await
                {
                    Ok(observation) => {
                        Self::log_observation(&observation, epoch, generation);
                        samples.push(observation.sample);
                    }
                    Err(ProbeFailure::ServerCooldown) => {
                        native_breaker::record_response_status_failure(
                            route,
                            429,
                            Some(policy::COOLDOWN),
                        );
                        return RoundStatus::NoGain;
                    }
                    Err(
                        ProbeFailure::TargetMissing | ProbeFailure::Cancelled,
                    ) => return RoundStatus::NoGain,
                    Err(_) => {}
                }
            }
        }
        let ranked =
            policy::rank_candidates(&samples, target.protocol, Instant::now());
        let Some(&(best, speed)) = ranked.first() else {
            return RoundStatus::NoGain;
        };
        let Some(incumbent) = incumbent else {
            return RoundStatus::NoGain;
        };
        let Some((_, current_speed)) =
            ranked.iter().find(|(ip, _)| *ip == incumbent)
        else {
            return RoundStatus::NoGain;
        };
        if best == incumbent
            || !policy::should_publish(*current_speed, speed, remaining)
        {
            return RoundStatus::NoGain;
        }
        let Some(preferred) = validated
            .iter()
            .find(|(c, _)| c.ip == best)
            .map(|(c, _)| c.clone())
        else {
            return RoundStatus::NoGain;
        };
        let backups = validated
            .into_iter()
            .map(|(c, _)| c)
            .filter(|c| c.ip != best)
            .collect();
        let selection = Selection {
            protocol: target.protocol,
            expires_at: preferred
                .expires_at
                .min(Instant::now() + policy::SELECTION_LIFETIME),
            preferred,
            backups,
            speed,
        };
        self.publish(selection, epoch, generation)
    }

    fn log_observation(
        observation: &ProbeObservation,
        epoch: u64,
        generation: u64,
    ) {
        tracing::debug!(authority = modrinth_cdn::HOST, epoch, generation,
            remote_ip = %observation.remote_addr, protocol = ?observation.sample.protocol,
            connection_id = observation.sample.connection_id, probe_bytes = observation.sample.bytes,
            body_elapsed_ms = observation.sample.elapsed.as_millis() as u64,
            ttfb_ms = observation.ttfb.as_millis() as u64,
            x_served_by = %observation.x_served_by, cache_status = %observation.cache_status,
            path_unknown = true, "Modrinth endpoint sample");
    }

    fn publish(
        &self,
        selection: Selection,
        epoch: u64,
        generation: u64,
    ) -> RoundStatus {
        let mut state = self.state.lock();
        state.registrations.retain(|_, token| !token.is_cancelled());
        if state.closed
            || state.epoch != epoch
            || state.generation != generation
            || state.registrations.is_empty()
            || selection.expires_at <= Instant::now()
            || state.round.as_ref().is_none_or(|r| {
                r.epoch != epoch
                    || r.generation != generation
                    || r.cancellation.is_cancelled()
            })
        {
            return RoundStatus::Stale;
        }
        state.previous = state.selected.take();
        state.selected = Some(selection);
        state.generation += 1;
        state.failures.clear();
        tracing::info!(authority = modrinth_cdn::HOST, epoch, generation = state.generation,
            preferred_ip = %state.selected.as_ref().unwrap().preferred.ip,
            throughput_bps = state.selected.as_ref().unwrap().speed, "Selected Modrinth endpoint");
        RoundStatus::Selected
    }

    #[cfg(test)]
    pub(crate) fn install_test_selection(
        &self,
        endpoint: SocketAddr,
        protocol: Protocol,
    ) {
        let mut state = self.state.lock();
        state.endpoint_port = endpoint.port();
        state.selected = Some(Selection {
            protocol,
            preferred: EndpointCandidate {
                ip: endpoint.ip(),
                source: super::model::CandidateSource::Verified,
                expires_at: Instant::now() + policy::SELECTION_LIFETIME,
            },
            backups: Vec::new(),
            speed: 5_000_000,
            expires_at: Instant::now() + policy::SELECTION_LIFETIME,
        });
        state.generation += 1;
    }
}

#[cfg(test)]
#[path = "tests/coordinator.rs"]
mod tests;
