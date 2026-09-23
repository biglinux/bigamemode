# 07 — Network and Telemetry

## 1. Position

This module **measures**. It does not tune.

That is a deliberate scope limit, not an omission. "Gaming network
optimization" is where placebo concentrates, and the brief names two traps
specifically. Both are designed against here, and the second one produced a
genuine bug in the previous code.

## 2. DNS latency is not game latency

A resolver's lookup time affects how long a *name* takes to resolve. Once the
game holds the server's address, the resolver plays no part in the round-trip
time of match traffic. The two are reported as separate things and the wording
is a function so it cannot drift between views:

```rust
pub fn dns_disclaimer() -> &'static str {
    "This measures how quickly each resolver answers a lookup. It does not \
     affect the round-trip time of match traffic once a game has connected."
}
```

### 2.1 How the measurement works

A minimal DNS client over UDP — query built by hand, response validated by
transaction id and RCODE, elapsed time measured across exactly one round trip.

Going through the system resolver instead would have measured the wrong thing:
its cache, its search domains, and whatever it does in parallel for AAAA.
A dozen **distinct** domains are queried rather than one repeatedly, because
querying one name repeatedly measures the resolver's cache, and a cold lookup is
what a game launcher actually performs.

Statistics are median and p95, never the mean. One stalled lookup drags a mean
above every sample actually observed; the tail is reported separately, where it
belongs. Jitter is computed **before** sorting, since it is inter-arrival
variation — sorting first would report 10, 20, 10, 20 ms as 3.3 ms of jitter
instead of 10.

### 2.2 Real result on the reference machine

```
DNS resolution benchmark (12 lookups each)
  System #1      median    38.5 ms   min   15.3   p95   168.2   jitter  45.0   loss 0%
  System #2      median    15.6 ms   min   12.4   p95    16.9   jitter   1.3   loss 0%
  Cloudflare     median    12.8 ms   min   11.2   p95   294.0   jitter  48.1   loss 0%
  Google         median    41.0 ms   min   30.1   p95    46.3   jitter   7.1   loss 0%
  Quad9          median    31.0 ms   min   10.5   p95   250.9   jitter  67.6   loss 0%
  OpenDNS        median   136.3 ms   min  134.0   p95   146.4   jitter   5.1   loss 0%

Lowest median lookup latency: Cloudflare
```

Note what the report does **not** say. It does not say Cloudflare will improve
anyone's ping. It names the metric it measured and stops.

The p95 column also earns its place: Cloudflare has the best median and a 294 ms
tail, while "System #2" is slower at the median and far steadier. A
median-only view would have hidden that, which is a good argument for showing
the user the numbers rather than only a winner.

## 3. Measure the link that carries the traffic

The reference machine has **42 network interfaces**: 12 ZeroTier, 1 Tailscale,
14 `veth`, 12 docker and libvirt bridges, loopback, and one Ethernet port. Any
feature that enumerates interfaces picks the wrong one here.

`primary_link()` follows the default route out of `/proc/net/route` — the kernel
table directly, so it does not depend on `ip` or `nmcli` being installed, nor on
their localised output. Verified:

```
link enp7s0 medium=Ethernet speed=1000Mb/s mtu=1500 gw=192.168.0.1
     qdisc=fq_codel modern=true
```

`primary_link_follows_the_default_route` asserts the answer is never a `veth`,
`br-` or `docker` device.

## 4. What is deliberately not applied

`has_modern_qdisc()` exists to let the UI say **"already good"** rather than
offer to enable something that is on. `fq_codel` is the kernel default on this
machine, and "we enabled fq_codel for you" would be a no-op reported as an
improvement — precisely the class of claim this project exists to stop making.

None of the following are applied, and each has a reason:

| Tweak | Why not |
|---|---|
| `fq_codel` / CAKE | already active; CAKE's benefit is at the gateway, not the host |
| Disable IPv6 | a myth; breaks services that need it |
| MTU 9000 | jumbo frames need end-to-end support; on a 1500-MTU path this causes fragmentation |
| BBR congestion control | TCP-only. Match traffic is overwhelmingly UDP, so this cannot affect it |
| Disable NIC offloads | raises CPU use to fix a problem that is rarely present |
| Wi-Fi power save off | genuinely useful on Wi-Fi — `NOT TESTED`, this machine is wired |

Wi-Fi power saving is the one item here with a real mechanism behind it, and it
is the one that could not be tested. It is left unimplemented rather than
shipped untested.

Anything that *is* eventually applied must clear the same four bars as every
Booster knob: detected, supported, reversible, verified.

## 5. Telemetry

### 5.1 The bug that was fixed

GPU telemetry walked `/sys/class/drm` by hand:

```rust
for card_entry in drm_dir.flatten() {
    let hwmon_dir = std::fs::read_dir(&hwmon_base).ok()?;   // returns from the FUNCTION
```

`/sys/class/drm` is full of entries with no `device/hwmon` — connector nodes
like `card1-DP-1`, plus `renderD*` and `version`. The first one encountered
returned `None` from the whole function. Readdir order is not stable, so GPU
temperature and clock appeared and vanished between runs.

And when it did complete, it returned the **first** readable card. On this bench
that is `card0`, the Cezanne integrated GPU that renders nothing — while the RX
9060 XT on `card1` is the card the user cares about and the only one exposing
`power1_average`.

Both go away by asking `hardware` which card matters:

```rust
pub async fn gpu_snapshot_for(gpu: Option<&hardware::Gpu>) -> GpuSnapshot
```

Verified live: `card1`, 48 °C, 26 W, 4% busy.

### 5.2 Polling

The Home tiles refresh every 2 seconds. Fast enough to feel live, slow enough
that an idle Dashboard is not competing with a game for CPU.

Two pollers were removed: the Profiles list rebuilt itself every 2 seconds
forever — with cover art that would mean re-reading the whole library twice a
second in a window nobody is looking at — and it now refreshes on navigation and
on an explicit button.

One remains and is recorded as unfinished: `dbus::service::run()` re-reads
`/tmp/falcond_status` every 500 ms for the process lifetime and diffs the whole
string. `inotify` on the file is the right mechanism. Audit finding DBUS-01,
carried into Known Limitations.

## 6. What is not built

No benchmark engine. See [09-BENCHMARKS.md](09-BENCHMARKS.md) for what exists,
what does not, and why the report says "not measured" rather than guessing.
