use super::endpoint_selection::tuning::{
    TRIALS, TrialKind, TrialLease, TrialRegistry,
};
use super::h2_pool::SharedH2Connection;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const INITIAL_CONNECTION_TARGET: usize = 4;
pub(crate) const MAX_CONNECTION_TARGET: usize = 32;
pub(crate) const STABILITY_WINDOW: Duration = Duration::from_millis(500);

fn next_target(current: usize) -> usize {
    (current * 2).min(MAX_CONNECTION_TARGET)
}

pub(crate) struct AuthorityScheduler {
    adaptive: Option<AdaptiveConnections>,
    byte_samples: HashMap<u64, usize>,
    monotonic_bytes: u64,
    connections: Vec<Arc<SharedH2Connection>>,
    target: usize,
    stable_since: Option<Instant>,
    cooldown_until: Option<Instant>,
    last_byte_sample: usize,
    last_byte_sample_at: Instant,
    connecting: Arc<tokio::sync::Semaphore>,
}

impl Default for AuthorityScheduler {
    fn default() -> Self {
        Self {
            adaptive: None,
            byte_samples: HashMap::new(),
            monotonic_bytes: 0,
            connections: Vec::new(),
            target: INITIAL_CONNECTION_TARGET,
            stable_since: None,
            cooldown_until: None,
            last_byte_sample: 0,
            last_byte_sample_at: Instant::now(),
            connecting: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}

impl AuthorityScheduler {
    pub(crate) fn prune(&mut self) {
        self.sample_connection_bytes();
        self.connections.retain(|connection| {
            if connection.is_retiring() {
                return false;
            }
            if connection.is_dead()
                || connection.is_idle_expired_for_scheduler()
            {
                connection.evict_for_scheduler();
                false
            } else {
                true
            }
        });
        if self.connections.is_empty() {
            self.target = if self.adaptive.is_some() {
                2
            } else {
                INITIAL_CONNECTION_TARGET
            };
            self.stable_since = None;
        }
    }

    pub(crate) fn add_connection(
        &mut self,
        connection: Arc<SharedH2Connection>,
    ) {
        if !connection.is_dead()
            && !connection.is_retiring()
            && !self
                .connections
                .iter()
                .any(|existing| Arc::ptr_eq(existing, &connection))
        {
            self.byte_samples.insert(
                connection.connection_id,
                connection.bytes_transferred_for_scheduler(),
            );
            self.connections.push(connection);
        }
    }

    pub(crate) fn least_loaded(&self) -> Option<Arc<SharedH2Connection>> {
        self.connections
            .iter()
            .filter(|connection| {
                !connection.is_dead() && !connection.is_retiring()
            })
            .min_by_key(|connection| {
                connection.assigned_streams_for_scheduler()
            })
            .cloned()
    }

    pub(crate) fn select_connection(
        &mut self,
    ) -> Option<Arc<SharedH2Connection>> {
        let connection = self.least_loaded()?;
        connection.reserve_stream_for_scheduler();
        Some(connection)
    }

    pub(crate) fn observe(&mut self, now: Instant) {
        self.sample_connection_bytes();
        if let Some(adaptive) = &mut self.adaptive {
            let busy = self.connections.iter().all(|connection| {
                connection.healthy_for_scheduler()
                    && connection.assigned_streams_for_scheduler() > 0
            }) && self
                .connections
                .iter()
                .map(|c| c.assigned_streams_for_scheduler())
                .sum::<usize>()
                > self.connections.len();
            let healthy = self
                .connections
                .iter()
                .all(|connection| connection.healthy_for_scheduler());
            let retire = adaptive.observe(
                now,
                self.monotonic_bytes,
                self.connections.len(),
                busy,
                healthy,
            );
            self.target = adaptive.target;
            if retire && let Some(connection) = self.connections.pop() {
                connection.retire();
            }
            return;
        }
        let total_bytes = self
            .connections
            .iter()
            .map(|connection| connection.bytes_transferred_for_scheduler())
            .sum();
        if now.duration_since(self.last_byte_sample_at) >= STABILITY_WINDOW {
            if total_bytes == self.last_byte_sample {
                self.stable_since = None;
                self.last_byte_sample_at = now;
                return;
            }
            self.last_byte_sample = total_bytes;
            self.last_byte_sample_at = now;
        }
        let Some(since) = self.stable_since else {
            self.stable_since = Some(now);
            return;
        };
        if self.connections.len() >= self.target
            && self.connections.iter().all(|connection| {
                connection.healthy_for_scheduler()
                    && connection.assigned_streams_for_scheduler() > 0
            })
            && now.duration_since(since) >= STABILITY_WINDOW
        {
            self.target = next_target(self.target);
            self.stable_since = None;
        }
    }

    pub(crate) fn target(&self) -> usize {
        self.target
    }

    pub(crate) fn connection_count(&self) -> usize {
        self.connections.len()
    }

    pub(crate) fn connections_for_eviction(
        &self,
    ) -> &[Arc<SharedH2Connection>] {
        &self.connections
    }

    pub(crate) fn can_expand(&self, now: Instant) -> bool {
        if let Some(adaptive) = &self.adaptive {
            let owns_trial = adaptive
                .trial
                .as_ref()
                .is_some_and(|trial| trial.lease.valid(now));
            if !owns_trial && TRIALS.snapshot(now).1.is_some() {
                return false;
            }
            if self
                .connections
                .iter()
                .map(|c| c.assigned_streams_for_scheduler())
                .sum::<usize>()
                <= self.connections.len()
            {
                return false;
            }
        }
        self.target <= MAX_CONNECTION_TARGET
            && self.connection_count() < self.target
            && self.cooldown_until.is_none_or(|until| now >= until)
    }

    pub(crate) fn reserve_connector(
        &self,
    ) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.connecting.clone().try_acquire_owned().ok()
    }

    pub(crate) fn rollback(&mut self, now: Instant) {
        if let Some(adaptive) = &mut self.adaptive {
            adaptive.rollback(now);
            self.target = adaptive.target;
            return;
        }
        self.target = (self.target / 2).max(INITIAL_CONNECTION_TARGET);
        self.stable_since = None;
        self.cooldown_until = Some(now + STABILITY_WINDOW * 4);
    }

    #[cfg(test)]
    pub(crate) fn targets() -> [usize; 4] {
        [4, 8, 16, 32]
    }

    pub(crate) fn enable_adaptive(&mut self, now: Instant) {
        if self.adaptive.is_none() {
            self.adaptive =
                Some(AdaptiveConnections::new(now, self.monotonic_bytes));
            self.target = 2;
        }
    }

    pub(crate) fn suspend_adaptive(&mut self, now: Instant) {
        if let Some(adaptive) = &mut self.adaptive {
            adaptive.rollback(now);
            self.target = adaptive.target;
            while self.connections.len() > self.target {
                self.connections.pop().unwrap().retire();
            }
        }
    }

    fn sample_connection_bytes(&mut self) {
        for connection in &self.connections {
            let current = connection.bytes_transferred_for_scheduler();
            let previous = self
                .byte_samples
                .entry(connection.connection_id)
                .or_insert(current);
            self.monotonic_bytes = self
                .monotonic_bytes
                .saturating_add(current.saturating_sub(*previous) as u64);
            *previous = current;
        }
        self.byte_samples.retain(|id, _| {
            self.connections
                .iter()
                .any(|connection| connection.connection_id == *id)
        });
    }
}

const ADAPTIVE_WINDOW: Duration = Duration::from_secs(3);
const ADAPTIVE_COOLDOWN: Duration = Duration::from_secs(30);

struct ConnectionTrial {
    lease: TrialLease,
    previous: usize,
    baseline_bps: u64,
    windows: u8,
    rate_sum: u64,
    attached: bool,
}

struct AdaptiveConnections {
    registry: Arc<TrialRegistry>,
    target: usize,
    sampled_at: Instant,
    sampled_bytes: u64,
    baseline_bps: Option<u64>,
    cooldown_until: Option<Instant>,
    trial: Option<ConnectionTrial>,
    revision: u64,
}

impl AdaptiveConnections {
    fn new(now: Instant, bytes: u64) -> Self {
        Self::with_registry(now, bytes, TRIALS.clone())
    }

    fn with_registry(
        now: Instant,
        bytes: u64,
        registry: Arc<TrialRegistry>,
    ) -> Self {
        let revision = registry.snapshot(now).0;
        Self {
            registry,
            target: 2,
            sampled_at: now,
            sampled_bytes: bytes,
            baseline_bps: None,
            cooldown_until: None,
            trial: None,
            revision,
        }
    }

    fn rollback(&mut self, now: Instant) {
        if let Some(trial) = self.trial.take() {
            self.target = trial.previous;
        }
        self.baseline_bps = None;
        self.cooldown_until = Some(now + ADAPTIVE_COOLDOWN);
    }

    /// Returns whether the added connection should retire. `bytes` is a
    /// monotonic delta sum, so pruning a connection cannot fabricate a rate.
    fn observe(
        &mut self,
        now: Instant,
        bytes: u64,
        count: usize,
        busy: bool,
        healthy: bool,
    ) -> bool {
        if self
            .trial
            .as_ref()
            .is_some_and(|trial| !trial.lease.valid(now))
        {
            self.rollback(now);
            self.sampled_at = now;
            self.sampled_bytes = bytes;
            return count > self.target;
        }
        let (revision, owner) = self.registry.snapshot(now);
        if self.trial.is_none()
            && (owner.is_some() || revision != self.revision)
        {
            self.revision = revision;
            self.baseline_bps = None;
            self.sampled_at = now;
            self.sampled_bytes = bytes;
            return false;
        }
        if let Some(trial) = &mut self.trial
            && !trial.attached
            && count >= self.target
        {
            trial.attached = true;
            self.sampled_at = now;
            self.sampled_bytes = bytes;
            return false;
        }
        let elapsed = now.saturating_duration_since(self.sampled_at);
        if elapsed < ADAPTIVE_WINDOW {
            return false;
        }
        let rate = (bytes.saturating_sub(self.sampled_bytes) as f64
            / elapsed.as_secs_f64()) as u64;
        self.sampled_at = now;
        self.sampled_bytes = bytes;
        if !busy || !healthy || rate == 0 {
            let added = self.trial.is_some();
            if added {
                self.rollback(now);
            } else {
                self.baseline_bps = None;
            }
            return added && count > self.target;
        }
        if let Some(trial) = &mut self.trial {
            if !trial.attached {
                return false;
            }
            trial.windows += 1;
            trial.rate_sum = trial.rate_sum.saturating_add(rate);
            if trial.windows < 2 {
                return false;
            }
            let gain = u128::from(trial.rate_sum / 2) * 100
                >= u128::from(trial.baseline_bps) * 110;
            if !gain {
                self.rollback(now);
                return count > self.target;
            }
            self.trial = None;
            self.baseline_bps = None;
            self.revision = self.registry.snapshot(now).0;
            return false;
        }
        if count < self.target
            || self.target >= 31
            || self.cooldown_until.is_some_and(|until| until > now)
        {
            return false;
        }
        let Some(baseline) = self.baseline_bps.replace(rate) else {
            return false;
        };
        if rate < baseline / 2 || rate > baseline.saturating_mul(2) {
            return false;
        }
        let Some(lease) = self.registry.acquire(
            TrialKind::Connections,
            now,
            Duration::from_secs(25),
        ) else {
            return false;
        };
        self.trial = Some(ConnectionTrial {
            lease,
            previous: self.target,
            baseline_bps: baseline.min(rate),
            windows: 0,
            rate_sum: 0,
            attached: false,
        });
        self.target += 1;
        false
    }
}

#[cfg(test)]
mod adaptive_tests {
    use super::*;

    #[test]
    fn adaptive_accepts_gain_rolls_back_no_gain_and_ignores_empty_queue() {
        // Isolated registries keep policy tests independent of concurrent
        // end-to-end endpoint trials in the same test process.
        for (rate, accepted) in [(1200, true), (1000, false)] {
            let start = Instant::now();
            let mut policy =
                AdaptiveConnections::with_registry(start, 0, Arc::default());
            policy.observe(start + ADAPTIVE_WINDOW, 3000, 2, true, true);
            policy.observe(start + ADAPTIVE_WINDOW * 2, 6000, 2, true, true);
            assert_eq!(policy.target, 3);
            policy.observe(start + ADAPTIVE_WINDOW * 2, 6000, 3, true, true);
            policy.observe(
                start + ADAPTIVE_WINDOW * 3,
                6000 + rate * 3,
                3,
                true,
                true,
            );
            let retire = policy.observe(
                start + ADAPTIVE_WINDOW * 4,
                6000 + rate * 6,
                3,
                true,
                true,
            );
            assert_eq!(policy.target, if accepted { 3 } else { 2 });
            assert_eq!(retire, !accepted);
            assert!(policy.trial.is_none());
        }
        let now = Instant::now();
        let mut policy =
            AdaptiveConnections::with_registry(now, 0, Arc::default());
        for n in 1..5 {
            policy.observe(now + ADAPTIVE_WINDOW * n, 0, 2, false, true);
        }
        assert_eq!(policy.target, 2);
        assert!(policy.trial.is_none());
    }

    #[tokio::test]
    async fn pruning_connections_never_subtracts_from_authority_payload() {
        async fn connection() -> Arc<SharedH2Connection> {
            let (client, _server) = tokio::io::duplex(65536);
            let (sender, _driver) =
                h2::client::handshake(client).await.unwrap();
            Arc::new(SharedH2Connection::for_test(sender))
        }
        let first = connection().await;
        let second = connection().await;
        let mut scheduler = AuthorityScheduler::default();
        scheduler.add_connection(first.clone());
        scheduler.add_connection(second.clone());
        first.record_bytes(1000);
        second.record_bytes(2000);
        scheduler.sample_connection_bytes();
        assert_eq!(scheduler.monotonic_bytes, 3000);
        second.evict_for_scheduler();
        scheduler.prune();
        first.record_bytes(500);
        scheduler.sample_connection_bytes();
        assert_eq!(scheduler.monotonic_bytes, 3500);
    }
}
