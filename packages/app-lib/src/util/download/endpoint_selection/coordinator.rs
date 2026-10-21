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
    active_feedback: HashMap<PoolKey, usize>,
    feedback: Option<FeedbackWindow>,
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

struct FeedbackWindow {
    generation: u64,
    started: Instant,
    bytes: u64,
    bad_windows: u8,
}

pub(crate) struct EndpointSelector {
    #[cfg(test)]
    pub(super) probe_fixture: Mutex<Option<ProbeFixture>>,
    mode: SelectionMode,
    config_scope: Arc<str>,
    state: Mutex<State>,
    probe_bytes: Arc<AtomicU64>,
    pub(super) pinned_clients: super::clients::PinnedClients,
}

#[cfg(test)]
pub(super) struct ProbeFixture {
    pub(super) candidates: Vec<(SocketAddr, reqwest::Certificate)>,
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
    trial: Option<super::tuning::TrialLease>,
    selector: Weak<EndpointSelector>,
    id: u64,
    status: watch::Sender<RoundStatus>,
    finished: bool,
}

impl RoundGuard {
    fn finish(mut self, status: RoundStatus) {
        tracing::debug!(authority = modrinth_cdn::HOST, round = self.id, reason = ?status, "Finished Modrinth endpoint trial");
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
            tracing::debug!(
                authority = modrinth_cdn::HOST,
                round = self.id,
                reason = "cancelled",
                "Finished Modrinth endpoint trial"
            );
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
    #[cfg(test)]
    pub(super) fn benchmark_counters(&self) -> (u64, u64, u64) {
        let state = self.state.lock();
        (
            self.probe_bytes.load(Ordering::Relaxed),
            state.generation,
            state.next_round,
        )
    }
    pub(crate) fn begin_feedback(&self, lease: &EndpointLease) {
        let mut state = self.state.lock();
        *state.active_feedback.entry(lease.pool.clone()).or_default() += 1;
    }

    pub(crate) fn end_feedback(&self, lease: &EndpointLease) {
        let mut state = self.state.lock();
        if let Some(count) = state.active_feedback.get_mut(&lease.pool) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.active_feedback.remove(&lease.pool);
            }
        }
        if !state.active_feedback.keys().any(|key| {
            key.generation == state.generation
                && key.network.epoch == state.epoch
        }) {
            // Gaps between files are idle time, not evidence of a slower
            // endpoint. Two bad windows must belong to continuous activity.
            state.feedback = None;
        }
    }
    pub(crate) fn observe_payload(
        &self,
        lease: &EndpointLease,
        bytes: u64,
        now: Instant,
        contaminated: bool,
    ) {
        let mut state = self.state.lock();
        if state.closed
            || state.epoch != lease.pool.network.epoch
            || state.generation != lease.pool.generation
            || state.selected.as_ref().is_none_or(|s| {
                s.preferred.ip != lease.pool.endpoint.ip()
                    || s.protocol != lease.pool.protocol
            })
        {
            return;
        }
        let previous_speed = state
            .previous
            .as_ref()
            .map_or(0, |selection| selection.speed);
        let generation = state.generation;
        let window = state.feedback.get_or_insert(FeedbackWindow {
            generation,
            started: now,
            bytes: 0,
            bad_windows: 0,
        });
        if window.generation != generation || contaminated {
            *window = FeedbackWindow {
                generation,
                started: now,
                bytes: 0,
                bad_windows: 0,
            };
            return;
        }
        window.bytes = window.bytes.saturating_add(bytes);
        let elapsed = now.saturating_duration_since(window.started);
        if elapsed < Duration::from_secs(3) {
            return;
        }
        let speed = (window.bytes as f64 / elapsed.as_secs_f64()) as u64;
        window.bad_windows =
            if speed.saturating_mul(100) < previous_speed.saturating_mul(80) {
                window.bad_windows.saturating_add(1)
            } else {
                0
            };
        window.bytes = 0;
        window.started = now;
        if window.bad_windows < 2 {
            return;
        }
        if let Some(previous) = state
            .previous
            .take()
            .filter(|s| s.expires_at > now && s.preferred.expires_at > now)
        {
            state.selected = Some(previous);
            state.generation += 1;
            state.cooldown_until = Some(now + policy::COOLDOWN);
            state.feedback = None;
            tracing::info!(
                authority = modrinth_cdn::HOST,
                generation = state.generation,
                reason = "rollback",
                "Reverted Modrinth endpoint after two poor production windows"
            );
        }
    }

    pub(crate) fn recovery_lease(
        &self,
        request: &DownloadRequest,
        route: &DownloadRoute,
        protocol: Protocol,
        current_peer: Option<SocketAddr>,
        current_speed: u64,
        remaining: u64,
    ) -> Option<EndpointLease> {
        if !request.integrity.supports_resume()
            || native_breaker::is_open(route)
        {
            return None;
        }
        let url = url::Url::parse(&route.url).ok()?;
        if !eligible_target(&url, &request.integrity) {
            return None;
        }
        let lease = self.lease(&url, protocol)?;
        if current_peer == Some(lease.pool.endpoint) {
            return None;
        }
        let speed = self.state.lock().selected.as_ref()?.speed;
        if !policy::should_publish(current_speed, speed, remaining) {
            return None;
        }
        let mut generations = request.endpoint_migration_generations.lock();
        let generation = (lease.pool.network.epoch, lease.pool.generation);
        if !generations.contains(&generation) {
            if generations.len() >= 2 {
                return None;
            }
            generations.insert(generation);
            request.endpoint_migrations.fetch_add(1, Ordering::AcqRel);
        }
        Some(lease)
    }
    pub(crate) fn new(scope: impl Into<Arc<str>>) -> Arc<Self> {
        Self::with_mode(scope, SelectionMode::configured())
    }

    pub(super) fn with_mode(
        scope: impl Into<Arc<str>>,
        mode: SelectionMode,
    ) -> Arc<Self> {
        Arc::new(Self {
            #[cfg(test)]
            probe_fixture: Mutex::new(None),
            pinned_clients: super::clients::PinnedClients::default(),
            mode,
            config_scope: scope.into(),
            probe_bytes: Arc::new(AtomicU64::new(0)),
            state: Mutex::new(State {
                active_feedback: HashMap::new(),
                feedback: None,
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
                    // reqwest can discover macOS proxies outside the env
                    // variables checked by the native transport. Until that
                    // OS lookup is shared, an unknown path cannot be pinned.
                    && !cfg!(target_os = "macos")
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
                trial: None,
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
        let Some(trial) = super::tuning::TRIALS.acquire(
            super::tuning::TrialKind::Endpoint,
            Instant::now(),
            policy::ROUND_TIMEOUT + Duration::from_secs(1),
        ) else {
            return;
        };
        let Some(mut start) = self.begin(Instant::now()) else {
            return;
        };
        start.guard.trial = Some(trial);
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
                #[cfg(test)]
                let fixture = selector.probe_fixture.lock().is_some();
                #[cfg(not(test))]
                let fixture = false;
                if !fixture
                    && !matches!(
                        crate::util::network::is_network_metered().await,
                        Ok(false)
                    )
                {
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
        let history = super::store::load().await;
        let candidates = collect_candidates(
            &answers,
            &verified,
            &history,
            true,
            Instant::now(),
            SystemTime::now(),
        );
        #[cfg(test)]
        let candidates = self
            .probe_fixture
            .lock()
            .as_ref()
            .map(|fixture| {
                fixture
                    .candidates
                    .iter()
                    .map(|(address, _)| EndpointCandidate {
                        ip: address.ip(),
                        source: super::model::CandidateSource::History,
                        expires_at: Instant::now() + policy::SELECTION_LIFETIME,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or(candidates);
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
            // A saturated authority may have only its reserved recovery
            // slot. Validate sequentially in that case instead of rejecting
            // every candidate while the first probe owns the only permit.
            .buffer_unordered(
                policy::VALIDATION_CONCURRENCY
                    .min(super::super::native_budget::available(route).max(1)),
            );
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
        // These are availability hints only. Save asynchronously so disk I/O
        // cannot consume the remaining network trial deadline.
        let hints = validated
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect();
        tokio::spawn(super::store::save(hints));
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
        let previous = validated.iter().find(|(c, _)| c.ip == incumbent).map(
            |(candidate, _)| Selection {
                protocol: target.protocol,
                preferred: candidate.clone(),
                backups: Vec::new(),
                speed: *current_speed,
                expires_at: candidate
                    .expires_at
                    .min(Instant::now() + policy::SELECTION_LIFETIME),
            },
        );
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
        self.publish_with_previous(selection, previous, epoch, generation)
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

    #[cfg(test)]
    fn publish(
        &self,
        selection: Selection,
        epoch: u64,
        generation: u64,
    ) -> RoundStatus {
        self.publish_with_previous(selection, None, epoch, generation)
    }

    fn publish_with_previous(
        &self,
        selection: Selection,
        previous: Option<Selection>,
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
        state.previous = state.selected.take().or(previous);
        state.feedback = None;
        state.selected = Some(selection);
        #[cfg(test)]
        if let Some(fixture) = self.probe_fixture.lock().as_ref()
            && let Some((address, _)) =
                fixture.candidates.iter().find(|(address, _)| {
                    Some(address.ip())
                        == state.selected.as_ref().map(|s| s.preferred.ip)
                })
        {
            state.endpoint_port = address.port();
        }
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

    #[cfg(test)]
    pub(crate) fn install_test_backup(&self, ip: IpAddr) {
        if let Some(selection) = &mut self.state.lock().selected {
            selection.backups.push(EndpointCandidate {
                ip,
                source: super::model::CandidateSource::Verified,
                expires_at: Instant::now() + policy::SELECTION_LIFETIME,
            });
        }
    }
}

#[cfg(test)]
#[path = "tests/coordinator.rs"]
mod tests;
