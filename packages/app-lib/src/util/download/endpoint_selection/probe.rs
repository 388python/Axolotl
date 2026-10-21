use super::model::{
    EndpointCandidate, ProbeFailure, ProbeTarget, Protocol, ThroughputSample,
};
use super::policy::{self, ProbeBudget};
use crate::util::download::native_budget::NativeBudgetPermit;
use crate::util::fetch::DownloadRoute;
use parking_lot::Mutex;
use reqwest::{StatusCode, Version, header};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

static NEXT_PROBE_CONNECTION: AtomicU64 = AtomicU64::new(1);

/// Field order is intentional: drop the client/pool before its physical
/// admission permit. Responses are local to a method borrowing this session.
pub(super) struct ProbeSession {
    client: reqwest::Client,
    _permit: NativeBudgetPermit,
    candidate: EndpointCandidate,
    target: ProbeTarget,
    bytes_received: Arc<AtomicU64>,
}

#[derive(Debug)]
pub(super) struct ProbeObservation {
    pub(super) sample: ThroughputSample,
    pub(super) remote_addr: std::net::SocketAddr,
    pub(super) ttfb: Duration,
    pub(super) x_served_by: String,
    pub(super) cache_status: String,
}

impl ProbeSession {
    pub(super) fn open(
        candidate: EndpointCandidate,
        target: ProbeTarget,
        route: &DownloadRoute,
        clients: &crate::util::fetch::DownloadClients,
        bytes_received: Arc<AtomicU64>,
    ) -> Result<Self, ProbeFailure> {
        use crate::util::fetch::ProxyPolicy;
        use crate::util::proxy::ProxyMode;
        if clients.ignore_ssl_errors
            || !crate::util::download::modrinth_cdn::is_current_url(&target.url)
            || !target.url.path().starts_with("/data/")
            || target.url.query().is_some()
            || target.size == 0
            || candidate.expires_at <= Instant::now()
            || !super::candidates::is_public_ip(candidate.ip)
            || (route.proxy == ProxyPolicy::System
                && (clients.proxy.mode == ProxyMode::Custom
                    || clients.proxy.mode == ProxyMode::System
                        && crate::util::download::native::system_proxy_configured()))
        {
            return Err(ProbeFailure::Ineligible);
        }
        let permit = crate::util::download::native_budget::try_acquire(route)
            .map_err(|_| ProbeFailure::BudgetExhausted)?;
        let client = probe_client_builder(&candidate, &target)
            .build()
            .map_err(|_| ProbeFailure::Transport)?;
        Ok(Self::with_client(
            candidate,
            target,
            client,
            permit,
            bytes_received,
        ))
    }

    fn with_client(
        candidate: EndpointCandidate,
        target: ProbeTarget,
        client: reqwest::Client,
        permit: NativeBudgetPermit,
        bytes_received: Arc<AtomicU64>,
    ) -> Self {
        Self {
            client,
            _permit: permit,
            candidate,
            target,
            bytes_received,
        }
    }

    pub(super) async fn validate_endpoint(
        &self,
        budget: &Mutex<ProbeBudget>,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<ProbeObservation, ProbeFailure> {
        self.request(
            policy::VALIDATION_BYTES,
            true,
            budget,
            cancellation,
            deadline.min(Instant::now() + policy::VALIDATION_TIMEOUT),
        )
        .await
    }

    pub(super) async fn sample_endpoint(
        &self,
        budget: &Mutex<ProbeBudget>,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<ProbeObservation, ProbeFailure> {
        self.request(
            policy::SAMPLE_BYTES,
            false,
            budget,
            cancellation,
            deadline.min(Instant::now() + policy::SAMPLE_TIMEOUT),
        )
        .await
    }

    async fn request(
        &self,
        wanted: u64,
        require_complete: bool,
        budget: &Mutex<ProbeBudget>,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<ProbeObservation, ProbeFailure> {
        let operation = async {
            if deadline <= Instant::now()
                || self.candidate.expires_at <= Instant::now()
            {
                return Err(ProbeFailure::Timeout);
            }
            let length = budget
                .lock()
                .reserve(wanted.min(self.target.size), Instant::now());
            if length == 0 {
                return Err(ProbeFailure::BudgetExhausted);
            }
            let started_at = Instant::now();
            let connection_id =
                NEXT_PROBE_CONNECTION.fetch_add(1, Ordering::Relaxed);
            let mut response = tokio::time::timeout_at(
                deadline.into(),
                self.client
                    .get(self.target.url.clone())
                    .header(header::RANGE, format!("bytes=0-{}", length - 1))
                    .header(header::ACCEPT_ENCODING, "identity")
                    .send(),
            )
            .await
            .map_err(|_| ProbeFailure::Timeout)?
            .map_err(classify_transport)?;
            let ttfb = started_at.elapsed();
            match response.status() {
                StatusCode::OK => return Err(ProbeFailure::RangeUnsupported),
                StatusCode::NOT_FOUND => {
                    return Err(ProbeFailure::TargetMissing);
                }
                StatusCode::TOO_MANY_REQUESTS
                | StatusCode::SERVICE_UNAVAILABLE => {
                    return Err(ProbeFailure::ServerCooldown);
                }
                status if status.is_redirection() => {
                    return Err(ProbeFailure::Redirect);
                }
                StatusCode::PARTIAL_CONTENT => {}
                _ => return Err(ProbeFailure::HttpStatus),
            }
            validate_headers(response.headers(), length, self.target.size)?;
            let remote_addr =
                response.remote_addr().ok_or(ProbeFailure::WrongPeer)?;
            if remote_addr.ip() != self.candidate.ip {
                return Err(ProbeFailure::WrongPeer);
            }
            let expected = match self.target.protocol {
                Protocol::Http1 => Version::HTTP_11,
                Protocol::H2 => Version::HTTP_2,
            };
            if response.version() != expected {
                return Err(ProbeFailure::WrongProtocol);
            }
            let x_served_by = bounded_header(response.headers(), "x-served-by");
            let cache_status = bounded_header(response.headers(), "x-cache");
            let body_started = Instant::now();
            let mut received = 0u64;
            let mut complete = false;
            loop {
                let chunk =
                    tokio::time::timeout_at(deadline.into(), response.chunk())
                        .await;
                match chunk {
                    Err(_) if !require_complete && received > 0 => break,
                    Err(_) => return Err(ProbeFailure::Timeout),
                    Ok(Err(error)) => {
                        return Err(if error.is_timeout() {
                            ProbeFailure::Timeout
                        } else {
                            ProbeFailure::Truncated
                        });
                    }
                    Ok(Ok(None)) => {
                        complete = true;
                        break;
                    }
                    Ok(Ok(Some(chunk))) => {
                        self.bytes_received
                            .fetch_add(chunk.len() as u64, Ordering::Relaxed);
                        received = received.saturating_add(chunk.len() as u64);
                        if received > length {
                            return Err(ProbeFailure::InvalidRange);
                        }
                    }
                }
            }
            if complete && received != length {
                return Err(ProbeFailure::Truncated);
            }
            if require_complete && !complete {
                return Err(ProbeFailure::Truncated);
            }
            let observed_at = Instant::now();
            Ok(ProbeObservation {
                sample: ThroughputSample {
                    ip: self.candidate.ip,
                    protocol: self.target.protocol,
                    connection_id,
                    bytes: received,
                    elapsed: observed_at
                        .saturating_duration_since(body_started),
                    observed_at,
                    complete_object: complete && received == self.target.size,
                    contaminated: false,
                },
                remote_addr,
                ttfb,
                x_served_by,
                cache_status,
            })
        };
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(ProbeFailure::Cancelled),
            result = operation => result,
        }
    }
}

fn probe_client_builder(
    candidate: &EndpointCandidate,
    target: &ProbeTarget,
) -> reqwest::ClientBuilder {
    let builder = crate::util::fetch::file_reqwest_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .danger_accept_invalid_certs(false)
        .https_only(true)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        // A session has exactly one outstanding request and no idle sockets.
        .pool_max_idle_per_host(0)
        .resolve(
            crate::util::download::modrinth_cdn::HOST,
            std::net::SocketAddr::new(
                candidate.ip,
                target.url.port_or_known_default().unwrap_or(443),
            ),
        );
    match target.protocol {
        Protocol::Http1 => builder.http1_only(),
        Protocol::H2 => builder.http2_prior_knowledge(),
    }
}

fn classify_transport(error: reqwest::Error) -> ProbeFailure {
    if error.is_timeout() {
        return ProbeFailure::Timeout;
    }
    let detail = format!("{error:?}").to_ascii_lowercase();
    if detail.contains("certificate") || detail.contains("certnotvalidforname")
    {
        ProbeFailure::InvalidCertificate
    } else if error.is_body() {
        ProbeFailure::Truncated
    } else {
        ProbeFailure::Transport
    }
}

fn validate_headers(
    headers: &header::HeaderMap,
    length: u64,
    total: u64,
) -> Result<(), ProbeFailure> {
    if headers
        .get(header::CONTENT_ENCODING)
        .is_some_and(|value| value != "identity")
    {
        return Err(ProbeFailure::InvalidEncoding);
    }
    let expected = format!("bytes 0-{}/{total}", length - 1);
    if headers
        .get(header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        != Some(expected.as_str())
        || headers
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            != Some(length)
    {
        return Err(ProbeFailure::InvalidRange);
    }
    Ok(())
}

fn bounded_header(headers: &header::HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(512)
        .collect()
}

#[cfg(test)]
#[path = "tests/probe.rs"]
mod tests;
