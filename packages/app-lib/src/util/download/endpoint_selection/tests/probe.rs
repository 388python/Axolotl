use super::*;
use crate::util::download::endpoint_selection::model::CandidateSource;
use crate::util::fetch::{DownloadRouteSource, ProxyPolicy};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use rustls_pki_types::PrivatePkcs8KeyDer;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Copy)]
enum ResponseKind {
    Valid,
    IgnoreRange,
    WrongTotal,
    Truncated,
    Encoded,
    Redirect,
    RateLimited,
    Missing,
    Stall,
}

struct TlsFixture {
    address: SocketAddr,
    root: reqwest::Certificate,
    requests: Arc<AtomicU64>,
    server: tokio::task::JoinHandle<()>,
    protocol: Protocol,
}

impl Drop for TlsFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl TlsFixture {
    async fn start(host: &str, kind: ResponseKind) -> Self {
        Self::start_protocol(host, kind, Protocol::Http1).await
    }

    async fn start_protocol(
        host: &str,
        kind: ResponseKind,
        protocol: Protocol,
    ) -> Self {
        Self::start_certificate(host, kind, protocol, false).await
    }

    async fn start_certificate(
        host: &str,
        kind: ResponseKind,
        protocol: Protocol,
        expired: bool,
    ) -> Self {
        let mut ca_params = CertificateParams::new(vec![]).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::from_params(&ca_params, &ca_key);
        let leaf_key = KeyPair::generate().unwrap();
        let mut params =
            CertificateParams::new(vec![host.to_string()]).unwrap();
        if expired {
            params.not_before = rcgen::date_time_ymd(2020, 1, 1);
            params.not_after = rcgen::date_time_ymd(2021, 1, 1);
        }
        let cert = params.signed_by(&leaf_key, &issuer).unwrap();
        let mut config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone(), ca.der().clone()],
                PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
            )
            .unwrap();
        if protocol == Protocol::H2 {
            config.alpn_protocols = vec![b"h2".to_vec()];
        }
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let root = reqwest::Certificate::from_der(ca.der()).unwrap();
        let requests = Arc::new(AtomicU64::new(0));
        let counted = requests.clone();
        let server = tokio::spawn(async move {
            let mut handlers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    incoming = listener.accept() => {
                        let Ok((socket, _)) = incoming else { break };
                        let acceptor = acceptor.clone();
                        let counted = counted.clone();
                        handlers.spawn(async move {
                            let Ok(mut stream) = acceptor.accept(socket).await else { return };
                            assert_eq!(stream.get_ref().1.server_name(), Some("cdn.modrinth.com"));
                            if protocol == Protocol::H2 {
                                let mut connection = h2::server::handshake(stream).await.unwrap();
                                let mut transfers = tokio::task::JoinSet::new();
                                while let Some(request) = connection.accept().await {
                                    let (request, mut respond) = request.unwrap();
                                    counted.fetch_add(1, Ordering::SeqCst);
                                    assert_eq!(request.uri().host(), Some("cdn.modrinth.com"));
                                    assert!(request.headers().get(header::AUTHORIZATION).is_none());
                                    assert!(request.headers().get(crate::util::fetch::DOWNLOAD_META_HEADER).is_none());
                                    let length = request.headers()[header::RANGE].to_str().unwrap()
                                        .strip_prefix("bytes=0-").unwrap().parse::<usize>().unwrap() + 1;
                                    let response = http::Response::builder().status(206)
                                        .header(header::CONTENT_RANGE, format!("bytes 0-{}/1048576", length - 1))
                                        .header(header::CONTENT_LENGTH, length)
                                        .header("x-served-by", "cache-h2-test")
                                        .body(()).unwrap();
                                    let mut stream = respond.send_response(response, false).unwrap();
                                    // The connection keeps polling while the task waits for
                                    // flow-control credit from the client.
                                    transfers.spawn(async move {
                                        let mut remaining = length;
                                        while remaining > 0 {
                                            let wanted = remaining.min(16 * 1024);
                                            stream.reserve_capacity(wanted);
                                            let Some(Ok(capacity)) = futures::future::poll_fn(|cx| stream.poll_capacity(cx)).await else { return };
                                            let sent = capacity.min(wanted);
                                            remaining -= sent;
                                            if stream.send_data(bytes::Bytes::from(vec![42; sent]), remaining == 0).is_err() { return; }
                                        }
                                    });
                                }
                                return;
                            }
                            let mut request = Vec::new();
                            loop {
                                let mut chunk = [0; 1024];
                                let Ok(read) = stream.read(&mut chunk).await else { return };
                                if read == 0 { return; }
                                request.extend_from_slice(&chunk[..read]);
                                if request.len() > 8192 { return; }
                                if request.windows(4).any(|w| w == b"\r\n\r\n") { break; }
                            }
                            counted.fetch_add(1, Ordering::SeqCst);
                            let request = String::from_utf8(request).unwrap();
                            assert!(request.contains("host: cdn.modrinth.com:"));
                            assert!(!request.contains("authorization:"));
                            assert!(!request.contains(crate::util::fetch::DOWNLOAD_META_HEADER));
                            let range = request.lines().find_map(|line| line.strip_prefix("range: bytes=0-")).unwrap();
                            let length = range.trim().parse::<u64>().unwrap() + 1;
                            let header = match kind {
                                ResponseKind::IgnoreRange => "HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\n\r\n".to_string(),
                                ResponseKind::Redirect => "HTTP/1.1 302 Found\r\nLocation: https://different.invalid/file\r\nContent-Length: 0\r\n\r\n".to_string(),
                                ResponseKind::RateLimited => "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 30\r\nContent-Length: 0\r\n\r\n".to_string(),
                                ResponseKind::Missing => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string(),
                                _ => format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {length}\r\n{}X-Served-By: cache-test\r\nX-Cache: HIT\r\nConnection: close\r\n\r\n",
                                    length - 1, if matches!(kind, ResponseKind::WrongTotal) { 999_999 } else { 1_048_576 },
                                    if matches!(kind, ResponseKind::Encoded) { "Content-Encoding: gzip\r\n" } else { "" }),
                            };
                            if stream.write_all(header.as_bytes()).await.is_err() { return; }
                            if matches!(kind, ResponseKind::IgnoreRange | ResponseKind::Stall) {
                                std::future::pending::<()>().await;
                            }
                            let length = if matches!(kind, ResponseKind::Truncated) { 1024 } else { length as usize };
                            let _ = stream.write_all(&vec![42; length]).await;
                            let _ = stream.shutdown().await;
                        });
                    }
                    result = handlers.join_next(), if !handlers.is_empty() => { result.unwrap().unwrap(); }
                }
            }
        });
        Self {
            address,
            root,
            requests,
            server,
            protocol,
        }
    }

    fn session(&self, trust_fixture: bool) -> ProbeSession {
        let target = ProbeTarget {
            url: url::Url::parse(&format!(
                "https://cdn.modrinth.com:{}/data/project/file.jar",
                self.address.port()
            ))
            .unwrap(),
            size: 1_048_576,
            protocol: self.protocol,
        };
        let candidate = EndpointCandidate {
            ip: self.address.ip(),
            source: CandidateSource::History,
            expires_at: Instant::now() + Duration::from_secs(60),
        };
        let route = DownloadRoute {
            url: target.url.to_string(),
            source: DownloadRouteSource::Official,
            is_mirror: false,
            allow_sensitive_headers: true,
            supports_range: true,
            proxy: ProxyPolicy::Direct,
        };
        let mut builder = probe_client_builder(&candidate, &target);
        if trust_fixture {
            builder = builder.add_root_certificate(self.root.clone());
        }
        ProbeSession::with_client(
            candidate,
            target,
            builder.build().unwrap(),
            crate::util::download::native_budget::try_acquire(&route).unwrap(),
            Arc::new(AtomicU64::new(0)),
        )
    }
}

#[tokio::test]
async fn validates_real_tls_host_sni_pinned_peer_and_exact_range() {
    let fixture =
        TlsFixture::start("cdn.modrinth.com", ResponseKind::Valid).await;
    let session = fixture.session(true);
    let result = session
        .validate_endpoint(
            &Mutex::new(ProbeBudget::new(Instant::now(), policy::ROUND_BYTES)),
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
    assert_eq!(result.remote_addr, fixture.address);
    assert_eq!(result.sample.bytes, policy::VALIDATION_BYTES);
    assert_eq!(result.x_served_by, "cache-test");
    assert_eq!(result.cache_status, "HIT");
    assert!(result.ttfb < Duration::from_secs(2));
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn validates_real_h2_tls_alpn_peer_and_bounded_payload() {
    let fixture = TlsFixture::start_protocol(
        "cdn.modrinth.com",
        ResponseKind::Valid,
        Protocol::H2,
    )
    .await;
    let session = fixture.session(true);
    let result = session
        .validate_endpoint(
            &Mutex::new(ProbeBudget::new(Instant::now(), policy::ROUND_BYTES)),
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
    assert_eq!(result.remote_addr, fixture.address);
    assert_eq!(result.sample.protocol, Protocol::H2);
    assert_eq!(result.sample.bytes, policy::VALIDATION_BYTES);
    assert_eq!(result.x_served_by, "cache-h2-test");
    assert_eq!(
        session.bytes_received.load(Ordering::Relaxed),
        policy::VALIDATION_BYTES
    );
}

#[tokio::test]
async fn wrong_hostname_and_untrusted_ca_fail_before_any_http_request() {
    for (host, trust, expired) in [
        ("different.invalid", true, false),
        ("cdn.modrinth.com", false, false),
        ("cdn.modrinth.com", true, true),
    ] {
        let fixture = TlsFixture::start_certificate(
            host,
            ResponseKind::Valid,
            Protocol::Http1,
            expired,
        )
        .await;
        let session = fixture.session(trust);
        let result = session
            .validate_endpoint(
                &Mutex::new(ProbeBudget::new(
                    Instant::now(),
                    policy::ROUND_BYTES,
                )),
                &CancellationToken::new(),
                Instant::now() + Duration::from_secs(2),
            )
            .await;
        assert_eq!(result.unwrap_err(), ProbeFailure::InvalidCertificate);
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn invalid_ranges_encoding_redirect_status_and_ignored_range_never_validate()
 {
    for (kind, expected) in [
        (ResponseKind::WrongTotal, ProbeFailure::InvalidRange),
        (ResponseKind::Encoded, ProbeFailure::InvalidEncoding),
        (ResponseKind::IgnoreRange, ProbeFailure::RangeUnsupported),
        (ResponseKind::Redirect, ProbeFailure::Redirect),
        (ResponseKind::RateLimited, ProbeFailure::ServerCooldown),
        (ResponseKind::Missing, ProbeFailure::TargetMissing),
        (ResponseKind::Truncated, ProbeFailure::Truncated),
    ] {
        let fixture = TlsFixture::start("cdn.modrinth.com", kind).await;
        let session = fixture.session(true);
        let result = session
            .validate_endpoint(
                &Mutex::new(ProbeBudget::new(
                    Instant::now(),
                    policy::ROUND_BYTES,
                )),
                &CancellationToken::new(),
                Instant::now() + Duration::from_secs(2),
            )
            .await;
        assert_eq!(result.unwrap_err(), expected);
        assert!(
            session.bytes_received.load(Ordering::Relaxed)
                <= policy::VALIDATION_BYTES
        );
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn stalled_body_and_cancelled_probe_have_bounded_lifetimes() {
    let fixture =
        TlsFixture::start("cdn.modrinth.com", ResponseKind::Stall).await;
    let session = fixture.session(true);
    let budget =
        Mutex::new(ProbeBudget::new(Instant::now(), policy::ROUND_BYTES));
    let result = session
        .validate_endpoint(
            &budget,
            &CancellationToken::new(),
            Instant::now() + Duration::from_millis(100),
        )
        .await;
    assert_eq!(result.unwrap_err(), ProbeFailure::Timeout);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let result = session
        .validate_endpoint(
            &budget,
            &cancellation,
            Instant::now() + Duration::from_secs(1),
        )
        .await;
    assert_eq!(result.unwrap_err(), ProbeFailure::Cancelled);
}

#[test]
fn header_checks_reject_wrong_bounds_unknown_total_short_or_missing_length() {
    for range in [
        "bytes 1-65536/1048576",
        "bytes 0-65534/1048576",
        "bytes 0-65535/*",
        "garbage",
    ] {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::CONTENT_RANGE, range.parse().unwrap());
        headers.insert(header::CONTENT_LENGTH, "65536".parse().unwrap());
        assert_eq!(
            validate_headers(&headers, 65536, 1048576),
            Err(ProbeFailure::InvalidRange)
        );
    }
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::CONTENT_RANGE,
        "bytes 0-65535/1048576".parse().unwrap(),
    );
    for length in [None, Some("1"), Some("invalid")] {
        headers.remove(header::CONTENT_LENGTH);
        if let Some(length) = length {
            headers.insert(header::CONTENT_LENGTH, length.parse().unwrap());
        }
        assert_eq!(
            validate_headers(&headers, 65536, 1048576),
            Err(ProbeFailure::InvalidRange)
        );
    }
}

#[tokio::test]
async fn empty_probe_budget_sends_no_request_and_counts_no_payload() {
    let fixture =
        TlsFixture::start("cdn.modrinth.com", ResponseKind::Valid).await;
    let session = fixture.session(true);
    let result = session
        .validate_endpoint(
            &Mutex::new(ProbeBudget::new(Instant::now(), 0)),
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(2),
        )
        .await;
    assert_eq!(result.unwrap_err(), ProbeFailure::BudgetExhausted);
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
    assert_eq!(session.bytes_received.load(Ordering::Relaxed), 0);
}

#[test]
fn production_probe_rejects_custom_proxy_insecure_tls_nonpublic_and_expired_candidates()
 {
    use crate::util::proxy::{ProxyConfig, ProxyMode};
    let config = ProxyConfig {
        mode: ProxyMode::Custom,
        url: "http://127.0.0.1:7890".into(),
        ..Default::default()
    };
    let clients =
        crate::util::fetch::DownloadClients::build(&config, false, false)
            .unwrap();
    let target = ProbeTarget {
        url: "https://cdn.modrinth.com/data/project/file.jar"
            .parse()
            .unwrap(),
        size: 1_048_576,
        protocol: Protocol::H2,
    };
    let candidate = EndpointCandidate {
        ip: "151.101.1.242".parse().unwrap(),
        source: CandidateSource::Verified,
        expires_at: Instant::now() + Duration::from_secs(60),
    };
    let route = DownloadRoute {
        url: target.url.to_string(),
        source: DownloadRouteSource::Official,
        is_mirror: false,
        allow_sensitive_headers: true,
        supports_range: true,
        proxy: ProxyPolicy::System,
    };
    for case in 0..4 {
        let mut clients = clients.clone();
        let mut candidate = candidate.clone();
        match case {
            0 => {}
            1 => {
                clients.proxy.mode = ProxyMode::None;
                clients.ignore_ssl_errors = true;
            }
            2 => {
                clients.proxy.mode = ProxyMode::None;
                candidate.ip = "127.0.0.1".parse().unwrap();
            }
            3 => {
                clients.proxy.mode = ProxyMode::None;
                candidate.expires_at = Instant::now();
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            ProbeSession::open(
                candidate,
                target.clone(),
                &route,
                &clients,
                Arc::new(AtomicU64::new(0))
            ),
            Err(ProbeFailure::Ineligible)
        ));
    }
}
