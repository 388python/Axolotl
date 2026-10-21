use super::model::{Protocol, ThroughputSample};
use crate::util::download::native_slow;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::time::{Duration, Instant};

pub(super) const MAX_CANDIDATES: usize = 16;
pub(super) const VALIDATION_CONCURRENCY: usize = 3;
pub(super) const VALIDATION_TIMEOUT: Duration = Duration::from_secs(3);
pub(super) const ROUND_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const VALIDATION_BYTES: u64 = 64 * 1024;
pub(super) const MAX_MEASURED_CANDIDATES: usize = 4;
pub(super) const SAMPLE_TIMEOUT: Duration = Duration::from_secs(3);
pub(super) const SAMPLE_BYTES: u64 = 4 * 1024 * 1024;
pub(super) const ROUND_BYTES: u64 = 32 * 1024 * 1024;
pub(super) const HOURLY_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const COOLDOWN: Duration = Duration::from_secs(60);
pub(super) const SELECTION_LIFETIME: Duration = Duration::from_secs(5 * 60);

pub(super) struct ProbeBudget {
    started_at: Instant,
    used: u64,
    limit: u64,
}

impl ProbeBudget {
    pub(super) fn new(now: Instant, limit: u64) -> Self {
        Self {
            started_at: now,
            used: 0,
            limit: limit.min(ROUND_BYTES),
        }
    }

    pub(super) fn remaining(&self, now: Instant) -> u64 {
        if now.saturating_duration_since(self.started_at) >= ROUND_TIMEOUT {
            0
        } else {
            self.limit.saturating_sub(self.used)
        }
    }

    /// Call under the round's short state lock before polling more payload.
    /// Wire overhead and bounded library preread are measured separately.
    pub(super) fn reserve(&mut self, requested: u64, now: Instant) -> u64 {
        let reserved = requested.min(self.remaining(now));
        self.used += reserved;
        reserved
    }

    pub(super) fn used(&self) -> u64 {
        self.used
    }
}

/// Two non-overlapping observations are needed to rank an endpoint's bulk
/// goodput. Very short complete objects do not prove sustainable bandwidth.
pub(super) fn rank_candidates(
    samples: &[ThroughputSample],
    protocol: Protocol,
    now: Instant,
) -> Vec<(IpAddr, u64)> {
    let mut grouped: HashMap<IpAddr, Vec<&ThroughputSample>> = HashMap::new();
    let mut seen = HashSet::new();
    for sample in samples {
        if sample.protocol != protocol
            || sample.contaminated
            || sample.bytes < VALIDATION_BYTES
            || sample.complete_object && sample.bytes < SAMPLE_BYTES
            || sample.elapsed.is_zero()
            || sample.elapsed < Duration::from_millis(250)
                && sample.bytes < SAMPLE_BYTES
            || sample.observed_at > now
            || now.saturating_duration_since(sample.observed_at)
                >= SELECTION_LIFETIME
            || !seen.insert((
                sample.ip,
                sample.connection_id,
                sample.observed_at,
            ))
        {
            continue;
        }
        grouped.entry(sample.ip).or_default().push(sample);
    }
    let mut ranked = Vec::new();
    for (ip, mut samples) in grouped {
        samples.sort_by_key(|sample| sample.observed_at);
        let mut rates: Vec<u64> = Vec::new();
        let mut previous_end = None;
        for sample in samples {
            let Some(start) = sample.observed_at.checked_sub(sample.elapsed)
            else {
                continue;
            };
            if previous_end.is_some_and(|end| start < end) {
                continue;
            }
            previous_end = Some(sample.observed_at);
            let rate = sample.bytes as f64 / sample.elapsed.as_secs_f64();
            if rate.is_finite() && rate >= 1.0 {
                rates.push(rate.min(u64::MAX as f64) as u64);
            }
        }
        if rates.len() < 2 {
            continue;
        }
        rates.sort_unstable();
        let slowest = rates[0];
        // Large disagreement means the trial is uncertain rather than fast.
        if rates[rates.len() - 1] > slowest.saturating_mul(2) {
            continue;
        }
        ranked.push((ip, slowest));
    }
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
}

pub(super) fn should_publish(
    current_speed: u64,
    candidate_speed: u64,
    remaining: u64,
) -> bool {
    native_slow::should_switch(
        current_speed,
        candidate_speed,
        remaining,
        remaining,
    )
}
