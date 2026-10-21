use crate::util::download_dns::DnsSource;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CandidateSource {
    Dns(DnsSource),
    Verified,
    History,
    FastlyRule(u32),
}

#[derive(Clone, Debug)]
pub(super) struct EndpointCandidate {
    pub(super) ip: IpAddr,
    pub(super) source: CandidateSource,
    pub(super) expires_at: Instant,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Protocol {
    Http1,
    H2,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct NetworkContext {
    pub(crate) config_scope: Arc<str>,
    pub(crate) epoch: u64,
}

/// Never use this connection identity as a native_budget or breaker key.
/// Those retain the existing authority/configuration identity across epochs.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PoolKey {
    pub(crate) authority: Arc<str>,
    pub(crate) network: NetworkContext,
    pub(crate) generation: u64,
    pub(crate) endpoint: SocketAddr,
    pub(crate) protocol: Protocol,
}

#[derive(Clone, Debug)]
pub(crate) struct EndpointLease {
    pub(crate) pool: PoolKey,
    pub(super) source: CandidateSource,
    pub(crate) expires_at: Instant,
}

/// A measurement is attributable to one connection and one protocol. An IP
/// alone does not identify a sample when several H2 streams share a socket.
#[derive(Clone, Debug)]
pub(super) struct ThroughputSample {
    pub(super) ip: IpAddr,
    pub(super) protocol: Protocol,
    pub(super) connection_id: u64,
    pub(super) bytes: u64,
    pub(super) elapsed: std::time::Duration,
    pub(super) observed_at: Instant,
    pub(super) complete_object: bool,
    pub(super) contaminated: bool,
}
