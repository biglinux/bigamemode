//! Network measurement.
//!
//! This module measures; it does not tune. That split is deliberate, because
//! "gaming network optimization" is where placebo concentrates, and two traps
//! matter in particular:
//!
//! * **DNS latency is not game latency.** A resolver's lookup time affects how
//!   long a name takes to resolve. Once the game has the server's address, the
//!   resolver has nothing to do with the round-trip time of match traffic. The
//!   two are reported as separate things here and must stay separate in the UI.
//! * **Measure the link that carries the traffic.** A desktop can have dozens
//!   of interfaces — Docker bridges, `ZeroTier`, Tailscale, veth pairs. Anything
//!   that enumerates interfaces picks the wrong one, so [`primary_link`] follows
//!   the default route instead.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::Path;
use std::time::{Duration, Instant};

/// Physical medium of a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medium {
    /// Wired Ethernet.
    Ethernet,
    /// Wireless.
    WiFi,
    /// Tunnel, bridge, or anything virtual.
    Virtual,
    /// Could not be determined.
    Unknown,
}

/// The interface that actually carries traffic to the internet.
#[derive(Debug, Clone)]
pub struct Link {
    /// Interface name, e.g. `enp7s0`.
    pub name: String,
    /// Medium.
    pub medium: Medium,
    /// Negotiated speed in Mb/s, when the driver reports one.
    pub speed_mbps: Option<u32>,
    /// MTU.
    pub mtu: Option<u32>,
    /// Default gateway.
    pub gateway: Option<IpAddr>,
    /// Queueing discipline on the root qdisc, e.g. `fq_codel`.
    pub qdisc: Option<String>,
}

impl Link {
    /// Whether the queueing discipline is one that actively manages latency
    /// under load.
    ///
    /// Reported so the UI can say "already good" rather than offering to enable
    /// something that is on: `fq_codel` is the default qdisc on most
    /// distribution kernels, and claiming credit for it would be exactly the
    /// kind of invented improvement this project refuses to make.
    #[must_use]
    pub fn has_modern_qdisc(&self) -> bool {
        self.qdisc
            .as_deref()
            .is_some_and(|q| matches!(q, "fq_codel" | "cake" | "fq" | "fq_pie"))
    }
}

/// Find the interface carrying the default route.
///
/// Returns `None` when there is no default route (no connectivity).
#[must_use]
pub fn primary_link() -> Option<Link> {
    let mut link = primary_link_brief()?;
    link.qdisc = root_qdisc(&link.name);
    Some(link)
}

/// [`primary_link`] without the queue discipline, which takes running `tc`:
/// only kernel files are read, so a live display can call it every tick.
#[must_use]
pub fn primary_link_brief() -> Option<Link> {
    let (name, gateway) = default_route()?;
    let sys = Path::new("/sys/class/net").join(&name);
    Some(Link {
        medium: medium_of(&name, &sys),
        speed_mbps: read_num(&sys.join("speed"))
            .filter(|s: &i64| *s > 0)
            .and_then(|s| u32::try_from(s).ok()),
        mtu: read_num(&sys.join("mtu")).and_then(|m: i64| u32::try_from(m).ok()),
        qdisc: None,
        gateway,
        name,
    })
}

/// Parse `/proc/net/route` for the default route's interface and gateway.
///
/// Reading the kernel table directly avoids depending on `ip`/`nmcli` being
/// installed, and avoids their localised output.
fn default_route() -> Option<(String, Option<IpAddr>)> {
    let content = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in content.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let iface = cols.next()?;
        let dest = cols.next()?;
        let gw = cols.next()?;
        if dest != "00000000" {
            continue; // not the default route
        }
        // Gateway is little-endian hex of an IPv4 address.
        let gateway = u32::from_str_radix(gw, 16)
            .ok()
            .filter(|v| *v != 0)
            .map(|v| IpAddr::from(std::net::Ipv4Addr::from(v.swap_bytes())));
        return Some((iface.to_owned(), gateway));
    }
    None
}

fn medium_of(name: &str, sys: &Path) -> Medium {
    if sys.join("wireless").exists() || sys.join("phy80211").exists() {
        return Medium::WiFi;
    }
    // A device symlink means a real bus device behind the interface; bridges,
    // veth and tunnels have none.
    if sys.join("device").exists() {
        return Medium::Ethernet;
    }
    if name.starts_with("lo") {
        Medium::Virtual
    } else {
        Medium::Unknown
    }
}

fn read_num<T: std::str::FromStr>(path: &Path) -> Option<T> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn root_qdisc(iface: &str) -> Option<String> {
    let out = std::process::Command::new("tc")
        .args(["qdisc", "show", "dev", iface])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains(" root "))?;
    line.split_whitespace().nth(1).map(str::to_owned)
}

// ── Latency statistics ───────────────────────────────────────────────────────

/// Summary of a set of latency samples.
#[derive(Debug, Clone, PartialEq)]
pub struct LatencyStats {
    /// Number of successful samples.
    pub samples: usize,
    /// Number of attempts that timed out or failed.
    pub lost: usize,
    /// Fastest sample, in milliseconds.
    pub min_ms: f64,
    /// Median. Reported instead of the mean because a single stalled lookup
    /// skews a mean badly and tells you nothing about the typical case.
    pub median_ms: f64,
    /// 95th percentile — the tail that is actually felt.
    pub p95_ms: f64,
    /// Mean absolute difference between consecutive samples.
    pub jitter_ms: f64,
}

impl LatencyStats {
    /// Compute statistics from raw samples and a count of failures.
    ///
    /// Returns `None` when nothing succeeded, because there is no honest
    /// summary of zero measurements.
    #[must_use]
    pub fn from_samples(mut samples_ms: Vec<f64>, lost: usize) -> Option<Self> {
        if samples_ms.is_empty() {
            return None;
        }
        // Jitter is inter-arrival variation, so it must be computed in the
        // order the samples were taken — before sorting.
        let jitter_ms = if samples_ms.len() < 2 {
            0.0
        } else {
            let total: f64 = samples_ms.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
            #[allow(clippy::cast_precision_loss)]
            let n = (samples_ms.len() - 1) as f64;
            total / n
        };

        samples_ms.sort_by(f64::total_cmp);
        Some(Self {
            min_ms: samples_ms[0],
            median_ms: crate::benchmark::percentile(&samples_ms, 50.0),
            p95_ms: crate::benchmark::percentile(&samples_ms, 95.0),
            jitter_ms,
            samples: samples_ms.len(),
            lost,
        })
    }
}

// ── DNS ──────────────────────────────────────────────────────────────────────

/// A resolver to benchmark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolver {
    /// Display name, e.g. `Cloudflare`.
    pub name: String,
    /// Address to query.
    pub address: IpAddr,
}

/// Result of benchmarking one resolver.
#[derive(Debug, Clone)]
pub struct DnsResult {
    /// The resolver.
    pub resolver: Resolver,
    /// Lookup latency, or `None` when every query failed.
    pub stats: Option<LatencyStats>,
}

/// Resolvers currently configured on the system, read from `/etc/resolv.conf`.
#[must_use]
pub fn system_resolvers() -> Vec<IpAddr> {
    let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|l| l.trim().strip_prefix("nameserver"))
        .filter_map(|a| a.trim().parse().ok())
        .collect()
}

/// Build a DNS query for an A record.
///
/// A hand-rolled query keeps this dependency-free and, more importantly, lets
/// the timing cover exactly one round trip rather than whatever the system
/// resolver decides to do (caching, search domains, parallel AAAA lookups).
#[must_use]
pub fn build_query(id: u16, domain: &str) -> Vec<u8> {
    let mut q = Vec::with_capacity(32 + domain.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&0x0100u16.to_be_bytes()); // standard query, recursion desired
    q.extend_from_slice(&1u16.to_be_bytes()); // one question
    q.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // no answer/authority/additional
    for label in domain.split('.').filter(|l| !l.is_empty()) {
        let len = u8::try_from(label.len().min(63)).unwrap_or(63);
        q.push(len);
        q.extend_from_slice(&label.as_bytes()[..len as usize]);
    }
    q.push(0); // root label
    q.extend_from_slice(&1u16.to_be_bytes()); // QTYPE A
    q.extend_from_slice(&1u16.to_be_bytes()); // QCLASS IN
    q
}

/// Check that a response is a reply to `id` and reports success.
#[must_use]
pub fn response_is_valid(response: &[u8], id: u16) -> bool {
    if response.len() < 12 {
        return false;
    }
    if u16::from_be_bytes([response[0], response[1]]) != id {
        return false;
    }
    let flags = u16::from_be_bytes([response[2], response[3]]);
    // QR bit must be set (it is a response) and RCODE must be 0 (no error).
    flags & 0x8000 != 0 && flags.trailing_zeros() >= 4
}

/// Time a single A-record lookup against one resolver.
///
/// Returns the elapsed milliseconds, or `None` on timeout or a bad reply.
#[must_use]
pub fn time_lookup(resolver: IpAddr, domain: &str, timeout: Duration) -> Option<f64> {
    let bind: SocketAddr = if resolver.is_ipv4() {
        "0.0.0.0:0".parse().ok()?
    } else {
        "[::]:0".parse().ok()?
    };
    let socket = UdpSocket::bind(bind).ok()?;
    socket.set_read_timeout(Some(timeout)).ok()?;
    socket.connect(SocketAddr::new(resolver, 53)).ok()?;

    // A fresh transaction id per query. It also defeats any cached response
    // being mistaken for a fresh one at the socket layer.
    #[allow(clippy::cast_possible_truncation)]
    let id = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos())) as u16;

    let query = build_query(id, domain);
    let start = Instant::now();
    socket.send(&query).ok()?;
    let mut buf = [0u8; 512];
    let n = socket.recv(&mut buf).ok()?;
    let elapsed = start.elapsed();

    if response_is_valid(&buf[..n], id) {
        Some(elapsed.as_secs_f64() * 1000.0)
    } else {
        None
    }
}

/// Benchmark one resolver over several domains.
///
/// Distinct domains are used on purpose: querying the same name repeatedly
/// measures the resolver's cache, not the resolver. A cold, uncached lookup is
/// what a game launcher actually performs.
#[must_use]
pub fn benchmark_resolver(resolver: &Resolver, domains: &[&str], timeout: Duration) -> DnsResult {
    let mut samples = Vec::with_capacity(domains.len());
    let mut lost = 0;
    for domain in domains {
        match time_lookup(resolver.address, domain, timeout) {
            Some(ms) => samples.push(ms),
            None => lost += 1,
        }
    }
    DnsResult {
        resolver: resolver.clone(),
        stats: LatencyStats::from_samples(samples, lost),
    }
}

/// The resolver with the lowest median lookup latency.
///
/// Returns `None` when nothing answered. The caller must describe this result
/// as what it is — a DNS lookup measurement — and must not present it as a
/// change to in-game latency.
#[must_use]
pub fn fastest(results: &[DnsResult]) -> Option<&DnsResult> {
    results.iter().filter(|r| r.stats.is_some()).min_by(|a, b| {
        let am = a.stats.as_ref().map_or(f64::MAX, |s| s.median_ms);
        let bm = b.stats.as_ref().map_or(f64::MAX, |s| s.median_ms);
        am.total_cmp(&bm)
    })
}

/// The one sentence that may be said about a DNS benchmark result.
///
/// Kept as a function so the wording cannot drift into a performance claim in
/// one view and not another.
#[must_use]
pub fn dns_disclaimer() -> &'static str {
    "This measures how quickly each resolver answers a lookup. It does not \
     affect the round-trip time of match traffic once a game has connected."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_resists_a_single_outlier() {
        // The reason median is reported rather than mean: one 900 ms stall
        // would drag a mean above every sample actually observed.
        let stats = LatencyStats::from_samples(vec![10.0, 11.0, 10.5, 900.0, 10.2], 0).unwrap();
        assert!(stats.median_ms < 12.0, "median was {}", stats.median_ms);
        assert!(
            (stats.p95_ms - 900.0).abs() < f64::EPSILON,
            "the tail is still reported"
        );
        assert!((stats.min_ms - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jitter_is_computed_before_sorting() {
        // Samples arriving 10, 20, 10, 20 have 10 ms of inter-sample variation.
        // Sorting first would give 10,10,20,20 and understate it as 3.3.
        let stats = LatencyStats::from_samples(vec![10.0, 20.0, 10.0, 20.0], 0).unwrap();
        assert!(
            (stats.jitter_ms - 10.0).abs() < 0.001,
            "jitter was {}",
            stats.jitter_ms
        );
    }

    #[test]
    fn a_steady_link_has_no_jitter() {
        let stats = LatencyStats::from_samples(vec![12.0, 12.0, 12.0], 0).unwrap();
        assert!(stats.jitter_ms.abs() < f64::EPSILON);
    }

    #[test]
    fn single_sample_has_zero_jitter() {
        let stats = LatencyStats::from_samples(vec![12.0], 0).unwrap();
        assert!(stats.jitter_ms.abs() < f64::EPSILON);
        assert_eq!(stats.samples, 1);
    }

    #[test]
    fn no_samples_yields_no_statistics() {
        // Zero measurements must not be summarised as zero latency.
        assert!(LatencyStats::from_samples(Vec::new(), 5).is_none());
    }

    #[test]
    fn dns_query_is_well_formed() {
        let q = build_query(0xBEEF, "example.com");
        assert_eq!(&q[0..2], &[0xBE, 0xEF], "transaction id");
        assert_eq!(&q[2..4], &[0x01, 0x00], "recursion desired");
        assert_eq!(&q[4..6], &[0x00, 0x01], "one question");
        // Labels: 7 "example" 3 "com" 0
        assert_eq!(q[12], 7);
        assert_eq!(&q[13..20], b"example");
        assert_eq!(q[20], 3);
        assert_eq!(&q[21..24], b"com");
        assert_eq!(q[24], 0);
        assert_eq!(&q[25..27], &[0x00, 0x01], "QTYPE A");
        assert_eq!(&q[27..29], &[0x00, 0x01], "QCLASS IN");
        assert_eq!(q.len(), 29);
    }

    #[test]
    fn query_handles_trailing_dots_and_subdomains() {
        let q = build_query(1, "a.b.c.");
        assert_eq!(q[12], 1);
        assert_eq!(q[14], 1);
        assert_eq!(q[16], 1);
        assert_eq!(q[18], 0, "root label terminates the name");
    }

    #[test]
    fn responses_are_matched_and_checked() {
        let ok = [0xBE, 0xEF, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
        assert!(response_is_valid(&ok, 0xBEEF));

        // Wrong transaction id — could be a stale or spoofed packet.
        assert!(!response_is_valid(&ok, 0x1234));

        // NXDOMAIN (RCODE 3): an answer, but not a successful lookup.
        let nx = [0xBE, 0xEF, 0x81, 0x83, 0, 1, 0, 0, 0, 0, 0, 0];
        assert!(!response_is_valid(&nx, 0xBEEF));

        // QR bit clear — that is a query, not a response.
        let query = [0xBE, 0xEF, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        assert!(!response_is_valid(&query, 0xBEEF));

        // Truncated garbage.
        assert!(!response_is_valid(&[0xBE], 0xBEEF));
        assert!(!response_is_valid(&[], 0xBEEF));
    }

    fn result(name: &str, median: Option<f64>) -> DnsResult {
        DnsResult {
            resolver: Resolver {
                name: name.into(),
                address: "1.1.1.1".parse().unwrap(),
            },
            stats: median.map(|m| LatencyStats {
                samples: 5,
                lost: 0,
                min_ms: m,
                median_ms: m,
                p95_ms: m,
                jitter_ms: 0.0,
            }),
        }
    }

    #[test]
    fn fastest_picks_the_lowest_median() {
        let results = vec![
            result("ISP", Some(19.2)),
            result("Cloudflare", Some(11.3)),
            result("Google", Some(14.8)),
        ];
        assert_eq!(fastest(&results).unwrap().resolver.name, "Cloudflare");
    }

    #[test]
    fn fastest_ignores_resolvers_that_never_answered() {
        let results = vec![result("Dead", None), result("Google", Some(14.8))];
        assert_eq!(fastest(&results).unwrap().resolver.name, "Google");
        assert!(fastest(&[result("Dead", None)]).is_none());
        assert!(fastest(&[]).is_none());
    }

    #[test]
    fn the_disclaimer_separates_lookup_time_from_match_latency() {
        let text = dns_disclaimer();
        assert!(text.contains("does not affect"));
        assert!(text.to_lowercase().contains("round-trip"));
    }

    #[test]
    fn modern_qdisc_detection() {
        let mut link = Link {
            name: "enp7s0".into(),
            medium: Medium::Ethernet,
            speed_mbps: Some(1000),
            mtu: Some(1500),
            gateway: None,
            qdisc: Some("fq_codel".into()),
        };
        assert!(link.has_modern_qdisc());
        link.qdisc = Some("cake".into());
        assert!(link.has_modern_qdisc());
        link.qdisc = Some("pfifo_fast".into());
        assert!(!link.has_modern_qdisc());
        link.qdisc = None;
        assert!(!link.has_modern_qdisc());
    }

    #[test]
    fn primary_link_follows_the_default_route() {
        // A machine can have many virtual interfaces (docker bridges,
        // ZeroTier, Tailscale, veth pairs); whatever comes back must be the
        // routed one, never a virtual device that merely happens to sort first.
        let Some(link) = primary_link() else {
            return; // no connectivity in this environment
        };
        assert!(!link.name.is_empty());
        assert!(!link.name.starts_with("veth"));
        assert!(!link.name.starts_with("br-"));
        assert!(!link.name.starts_with("docker"));
        assert_ne!(link.medium, Medium::Virtual);
    }
}
