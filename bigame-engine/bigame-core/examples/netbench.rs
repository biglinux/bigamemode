//! Measure the primary link and benchmark DNS resolvers on this connection.
use bigame_core::network::{self, Resolver};
use std::time::Duration;

fn main() {
    match network::primary_link() {
        Some(l) => println!(
            "link {} medium={:?} speed={:?}Mb/s mtu={:?} gw={:?} qdisc={:?} modern={}",
            l.name,
            l.medium,
            l.speed_mbps,
            l.mtu,
            l.gateway,
            l.qdisc,
            l.has_modern_qdisc()
        ),
        None => println!("no default route"),
    }
    println!("system resolvers: {:?}", network::system_resolvers());

    // Distinct, unlikely-to-be-cached names: querying one name repeatedly
    // measures the resolver's cache instead of the resolver.
    let domains = [
        "steamcommunity.com",
        "store.steampowered.com",
        "epicgames.com",
        "gog.com",
        "ea.com",
        "ubisoft.com",
        "battle.net",
        "riotgames.com",
        "playstation.com",
        "xbox.com",
        "nvidia.com",
        "amd.com",
    ];

    let mut candidates: Vec<Resolver> = Vec::new();
    for (i, ip) in network::system_resolvers().into_iter().enumerate() {
        candidates.push(Resolver {
            name: format!("System #{}", i + 1),
            address: ip,
        });
    }
    for (name, ip) in [
        ("Cloudflare", "1.1.1.1"),
        ("Google", "8.8.8.8"),
        ("Quad9", "9.9.9.9"),
        ("OpenDNS", "208.67.222.222"),
    ] {
        let addr = ip.parse().unwrap();
        if !candidates.iter().any(|c| c.address == addr) {
            candidates.push(Resolver {
                name: name.into(),
                address: addr,
            });
        }
    }

    let results: Vec<_> = candidates
        .iter()
        .map(|r| network::benchmark_resolver(r, &domains, Duration::from_secs(2)))
        .collect();

    println!(
        "\nDNS resolution benchmark ({} lookups each)",
        domains.len()
    );
    for r in &results {
        match &r.stats {
            Some(s) => println!(
                "  {:<14} median {:>7.1} ms   min {:>6.1}   p95 {:>7.1}   jitter {:>6.1}   loss {:.0}%",
                r.resolver.name,
                s.median_ms,
                s.min_ms,
                s.p95_ms,
                s.jitter_ms,
                s.loss_ratio() * 100.0
            ),
            None => println!("  {:<14} no response", r.resolver.name),
        }
    }
    if let Some(best) = network::fastest(&results) {
        println!("\nLowest median lookup latency: {}", best.resolver.name);
        println!("{}", network::dns_disclaimer());
    }
}
