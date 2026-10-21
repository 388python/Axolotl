use super::super::model::CandidateSource;
use super::*;

fn selection(ip: &str, now: Instant) -> Selection {
    Selection {
        protocol: Protocol::H2,
        preferred: EndpointCandidate {
            ip: ip.parse().unwrap(),
            source: CandidateSource::Verified,
            expires_at: now + policy::SELECTION_LIFETIME,
        },
        backups: vec![EndpointCandidate {
            ip: "151.101.65.242".parse().unwrap(),
            source: CandidateSource::Verified,
            expires_at: now + policy::SELECTION_LIFETIME,
        }],
        speed: 5_000_000,
        expires_at: now + policy::SELECTION_LIFETIME,
    }
}

#[test]
fn mode_defaults_and_unknown_values_are_passive() {
    assert_eq!(SelectionMode::parse(None), SelectionMode::Off);
    assert_eq!(SelectionMode::parse(Some("AUTO")), SelectionMode::Off);
    assert_eq!(
        SelectionMode::parse(Some("observe")),
        SelectionMode::Observe
    );
    for mode in [SelectionMode::Off, SelectionMode::Observe] {
        let selector = EndpointSelector::with_mode("test", mode);
        let _registration = selector.register(CancellationToken::new());
        assert!(selector.begin(Instant::now()).is_none());
        assert!(
            selector
                .lease(
                    &"https://cdn.modrinth.com/data/a".parse().unwrap(),
                    Protocol::H2
                )
                .is_none()
        );
        assert_eq!(selector.probe_bytes.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn twenty_downloads_share_one_round_and_one_cancellation_does_not_cancel_others()
 {
    let selector = EndpointSelector::with_mode("test", SelectionMode::Auto);
    let mut guards: Vec<_> = (0..20)
        .map(|_| selector.register(CancellationToken::new()).unwrap())
        .collect();
    let start = selector.begin(Instant::now()).unwrap();
    let mut status = start.guard.status.subscribe();
    for _ in 0..20 {
        assert!(selector.begin(Instant::now()).is_none());
    }
    guards.pop();
    assert!(!start.cancellation.is_cancelled());
    drop(guards);
    assert!(start.cancellation.is_cancelled());
    drop(start);
    assert_eq!(*status.borrow_and_update(), RoundStatus::Cancelled);
    assert!(selector.state.lock().round.is_none());
}

#[test]
fn cancelled_or_unpolled_worker_retains_terminal_status_and_releases_slot() {
    let selector = EndpointSelector::with_mode("test", SelectionMode::Auto);
    let _registration = selector.register(CancellationToken::new()).unwrap();
    let start = selector.begin(Instant::now()).unwrap();
    let status = start.guard.status.subscribe();
    drop(start);
    assert_eq!(*status.borrow(), RoundStatus::Cancelled);
    assert!(selector.state.lock().round.is_none());
    assert!(selector.state.lock().cooldown_until.is_some());
}

#[test]
fn publication_requires_live_epoch_generation_and_observers() {
    let selector = EndpointSelector::with_mode("test", SelectionMode::Auto);
    let _registration = selector.register(CancellationToken::new()).unwrap();
    let start = selector.begin(Instant::now()).unwrap();
    let now = Instant::now();
    assert_eq!(
        selector.publish(
            selection("146.75.113.242", now),
            start.epoch + 1,
            start.generation
        ),
        RoundStatus::Stale
    );
    assert_eq!(
        selector.publish(
            selection("146.75.113.242", now),
            start.epoch,
            start.generation + 1
        ),
        RoundStatus::Stale
    );
    assert_eq!(
        selector.publish(
            selection("146.75.113.242", now),
            start.epoch,
            start.generation
        ),
        RoundStatus::Selected
    );
    let lease = selector
        .lease(
            &"https://cdn.modrinth.com/data/a".parse().unwrap(),
            Protocol::H2,
        )
        .unwrap();
    assert_eq!(lease.pool.endpoint.ip().to_string(), "146.75.113.242");
    assert!(selector.is_current(&lease));
    assert!(
        selector
            .lease(&"https://api.modrinth.com/".parse().unwrap(), Protocol::H2)
            .is_none()
    );
    assert!(
        selector
            .lease(
                &"https://cdn.modrinth.com/".parse().unwrap(),
                Protocol::Http1
            )
            .is_none()
    );
    selector.close();
    assert!(!selector.is_current(&lease));
    assert!(start.cancellation.is_cancelled());
    assert_eq!(
        selector.publish(
            selection("151.101.1.242", now),
            start.epoch,
            start.generation
        ),
        RoundStatus::Stale
    );
}

#[test]
fn failed_endpoint_moves_to_validated_backup_without_resetting_hourly_budget() {
    let selector = EndpointSelector::with_mode("test", SelectionMode::Auto);
    let _registration = selector.register(CancellationToken::new()).unwrap();
    let start = selector.begin(Instant::now()).unwrap();
    assert_eq!(
        selector.publish(
            selection("146.75.113.242", Instant::now()),
            start.epoch,
            start.generation
        ),
        RoundStatus::Selected
    );
    let url = "https://cdn.modrinth.com/data/a".parse().unwrap();
    let lease = selector.lease(&url, Protocol::H2).unwrap();
    selector.record_failure(&lease, false);
    assert!(selector.is_current(&lease));
    selector.record_failure(&lease, false);
    let backup = selector.lease(&url, Protocol::H2).unwrap();
    assert_eq!(backup.pool.endpoint.ip().to_string(), "151.101.65.242");
    assert!(!selector.is_current(&lease));
    assert_eq!(selector.state.lock().hourly_reservations.len(), 1);
    selector.record_failure(&lease, true);
    assert!(selector.is_current(&backup));
}

#[test]
fn cancelled_trials_keep_hourly_reservations_and_expiry_cannot_republish_old_epoch()
 {
    let selector = EndpointSelector::with_mode("test", SelectionMode::Auto);
    let _registration = selector.register(CancellationToken::new()).unwrap();
    for _ in 0..2 {
        selector.state.lock().cooldown_until = None;
        let round = selector.begin(Instant::now()).unwrap();
        drop(round);
    }
    selector.state.lock().cooldown_until = None;
    assert!(selector.begin(Instant::now()).is_none());
    let mut state = selector.state.lock();
    let epoch = state.epoch;
    state.last_activity = Instant::now() - policy::SELECTION_LIFETIME;
    EndpointSelector::expire(&mut state, Instant::now());
    assert_ne!(state.epoch, epoch);
    assert_eq!(state.hourly_reservations.len(), 2);
}

#[tokio::test]
async fn saturated_production_leaves_exactly_one_recovery_slot_across_generations()
 {
    use crate::util::download::{native_budget, proxy_context};
    let mut clients = DownloadClients::build(
        &crate::util::proxy::ProxyConfig {
            mode: ProxyMode::None,
            ..Default::default()
        },
        false,
        false,
    )
    .unwrap();
    clients.scope = "reserved-slot-test".into();
    clients.endpoints =
        EndpointSelector::with_mode(clients.scope.clone(), SelectionMode::Auto);
    let selector = clients.endpoints.clone();
    let route = DownloadRoute {
        url: "https://cdn.modrinth.com/data/a".into(),
        source: crate::util::fetch::DownloadRouteSource::Official,
        is_mirror: false,
        allow_sensitive_headers: true,
        supports_range: true,
        proxy: ProxyPolicy::System,
    };
    proxy_context::with_snapshot(clients, async {
        let mut regular = Vec::new();
        for _ in 0..31 {
            regular.push(native_budget::try_acquire(&route).unwrap());
        }
        selector.install_test_selection(
            "151.101.1.242:443".parse().unwrap(),
            Protocol::H2,
        );
        assert!(native_budget::try_acquire(&route).is_err());
        let recovery = native_budget::try_acquire_recovery(&route).unwrap();
        assert_eq!(native_budget::available(&route), 0);
        assert!(native_budget::try_acquire_recovery(&route).is_err());
        drop(recovery);
        drop(regular);
        assert_eq!(native_budget::available(&route), 32);
    })
    .await;
}

#[test]
fn same_url_new_endpoint_recovers_only_with_integrity_and_shared_migration_budget()
 {
    let selector =
        EndpointSelector::with_mode("recovery-test", SelectionMode::Auto);
    selector.install_test_selection(
        "151.101.1.242:443".parse().unwrap(),
        Protocol::H2,
    );
    let request = DownloadRequest::new(
        "https://cdn.modrinth.com/data/project/file.jar",
        crate::util::fetch::ResourceClass::Modrinth,
    )
    .with_integrity(
        crate::util::fetch::Integrity::sha1("test").with_size(1024 * 1024),
    );
    let route = DownloadRoute {
        url: request.url.clone(),
        source: crate::util::fetch::DownloadRouteSource::Official,
        is_mirror: false,
        allow_sensitive_headers: true,
        supports_range: true,
        proxy: ProxyPolicy::Direct,
    };
    let previous = Some("151.101.65.242:443".parse().unwrap());
    let clone = request.clone();
    for request in [&request, &clone] {
        let lease = selector
            .recovery_lease(
                request,
                &route,
                Protocol::H2,
                previous,
                45 * 1024,
                512 * 1024,
            )
            .unwrap();
        assert_eq!(lease.pool.endpoint.ip().to_string(), "151.101.1.242");
    }
    assert!(
        selector
            .recovery_lease(
                &request,
                &route,
                Protocol::H2,
                previous,
                45 * 1024,
                512 * 1024
            )
            .is_none()
    );
    let fresh = DownloadRequest::new(request.url.clone(), request.resource)
        .with_integrity(request.integrity.clone());
    assert!(
        selector
            .recovery_lease(
                &fresh,
                &route,
                Protocol::H2,
                Some("151.101.1.242:443".parse().unwrap()),
                45 * 1024,
                512 * 1024
            )
            .is_none()
    );
    assert!(
        selector
            .recovery_lease(
                &DownloadRequest::new(request.url, request.resource),
                &route,
                Protocol::H2,
                previous,
                45 * 1024,
                512 * 1024
            )
            .is_none()
    );
}
