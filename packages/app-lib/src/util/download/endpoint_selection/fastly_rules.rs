use super::model::{CandidateSource, EndpointCandidate};
use crate::util::download::modrinth_cdn;
use crate::util::download_dns::{DnsAnswer, DnsSource};
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CATALOG_VERSION: u32 = 1;
// Reviewed 2026-10-21 against the public A/AAAA and CNAME responses. These
// prefixes remain untrusted seeds; every use requires TLS and Range checks.
const REVIEWED_UNIX_SECONDS: u64 = 1_792_540_800;
const CATALOG_LIFETIME: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const KNOWN_CNAME: &str = "72b69f.atlas.fastly.net";
const POOL_THIRD_OCTETS: [u8; 4] = [1, 65, 129, 193];
const SEED_PREFIXES: [[u8; 3]; 3] =
    [[146, 75, 113], [151, 101, 41], [146, 75, 93]];

pub(super) fn seed_candidates(
    answers: &[DnsAnswer],
    now: Instant,
    wall_time: SystemTime,
) -> Vec<EndpointCandidate> {
    let reviewed = UNIX_EPOCH + Duration::from_secs(REVIEWED_UNIX_SECONDS);
    let expires = reviewed + CATALOG_LIFETIME;
    if wall_time < reviewed || wall_time >= expires {
        return Vec::new();
    }
    let fresh: Vec<_> = answers
        .iter()
        .filter(|answer| answer.addresses.iter().any(|a| a.expires_at > now))
        .collect();
    if fresh.is_empty()
        || fresh.iter().any(|answer| {
            answer.queried_host != modrinth_cdn::HOST
                || !matches!(answer.source, DnsSource::Doh(_))
                || answer.cname_chain.as_slice() != [KNOWN_CNAME]
        })
    {
        return Vec::new();
    }
    let mut suffix = None;
    let mut seen = [false; 4];
    let mut valid_until = now + Duration::from_secs(5 * 60);
    for answer in fresh {
        for address in answer.addresses.iter().filter(|a| a.expires_at > now) {
            valid_until = valid_until.min(address.expires_at);
            let IpAddr::V4(ip) = address.ip else { continue };
            let [a, b, c, d] = ip.octets();
            let Some(index) =
                POOL_THIRD_OCTETS.iter().position(|&octet| octet == c)
            else {
                return Vec::new();
            };
            if a != 151 || b != 101 || suffix.is_some_and(|suffix| suffix != d)
            {
                return Vec::new();
            }
            suffix = Some(d);
            seen[index] = true;
        }
    }
    let Some(suffix) = suffix.filter(|_| seen.into_iter().all(|value| value))
    else {
        return Vec::new();
    };
    valid_until =
        valid_until.min(now + expires.duration_since(wall_time).unwrap());
    SEED_PREFIXES
        .into_iter()
        .map(|[a, b, c]| EndpointCandidate {
            ip: Ipv4Addr::new(a, b, c, suffix).into(),
            source: CandidateSource::FastlyRule(CATALOG_VERSION),
            expires_at: valid_until,
        })
        .collect()
}
