# 04 — Test Matrix

**Rule: compilation is not compatibility.** Anything that could not be exercised
on real hardware is marked `NOT TESTED — hardware unavailable`, and the number of
those rows is the honest measure of this matrix.

Reference machine (BENCH-1) is described in [00-BASELINE.md](00-BASELINE.md):
BigLinux, kernel 7.2.6-xanmod, Wayland/KDE, Ryzen 7 5700G, RX 9060 XT + Cezanne
iGPU, Ethernet, desktop.

---

## 1. Automated

| Command | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo check --workspace` | pass |
| `cargo test --workspace` | **217 passed, 0 failed** |
| `cargo clippy --workspace --all-targets` | pass; **0 warnings in new modules** |

217 tests, up from 77 at the branch point. Pre-existing pedantic warnings remain
in untouched UI files and are listed as a known limitation rather than silenced.

### 1.1 Test hygiene

Two hermeticity problems were found and one was fixed.

**Fixed.** The journal tests mutated `XDG_STATE_HOME`, which is process-global,
while `cargo test` runs them in parallel threads. They raced, failed
non-deterministically, and one run leaked a journal into the real
`~/.local/state/bigame-mode/` — which the UI then displayed as "Booster Mode
Active, 0 optimizations". `Journal::save_to`/`load_from` now take explicit paths
and the tests use private fixtures.

**Known, not fixed.** `test_launch_plan_*` gate on `is_turbo_mode_active()`,
which falls through to live PowerProfiles D-Bus. They pass here only because the
machine sits in `performance`; on a machine in `balanced` they fail. Audit
finding T-01. `video_config` tests mutate `XDG_CONFIG_HOME` the same way the
journal tests used to.

---

## 2. Hardware and platform

| Dimension | Status | Evidence |
|---|---|---|
| **Wayland** | Tested | session type `wayland`, KDE; UI runs, screenshots taken |
| **X11** | NOT TESTED — no X11 session | detection path unit-tested |
| **AMD GPU (discrete)** | Tested | RX 9060 XT, Navi 44, RADV, Mesa 26.2.2 |
| **AMD GPU (integrated)** | Tested | Cezanne iGPU present and correctly *not* selected |
| **Dual GPU** | Tested | render-GPU selection verified; this is what exposed TEL-02 |
| **NVIDIA GPU** | NOT TESTED — hardware unavailable | `nvidia-smi` fallback path is code-reviewed only |
| **Intel GPU** | NOT TESTED — hardware unavailable | vendor mapping unit-tested |
| **AMD CPU** | Tested | Ryzen 7 5700G, Zen 3, `amd-pstate-epp` |
| **Intel CPU** | NOT TESTED — hardware unavailable | vendor parsing unit-tested |
| **Hybrid P/E cores** | NOT TESTED — hardware unavailable | detection by max-frequency spread; logic only |
| **AMD 3D V-Cache** | NOT TESTED — hardware unavailable | correctly reported absent; sysfs glob unverified |
| **Desktop** | Tested | `chassis_type` → desktop |
| **Laptop / battery** | NOT TESTED — hardware unavailable | battery refusal path unit-tested |
| **Handheld** | NOT TESTED — hardware unavailable | — |
| **VRR display** | NOT TESTED — hardware unavailable | both outputs report `Vrr: incapable` |
| **HDR display** | NOT TESTED — hardware unavailable | both outputs report `HDR: incapable` |
| **Ethernet** | Tested | enp7s0, 1 Gb/s, fq_codel |
| **Wi-Fi** | NOT TESTED — hardware unavailable | medium detection unit-tested |

---

## 3. Booster Mode

| Case | Status | Evidence |
|---|---|---|
| Detect hardware | Tested | correct CPU, both GPUs, render GPU = card1, 3 displays |
| Detect capabilities | Tested | gamescope 3.16.28/97 flags, PPD profiles, 16 schedulers, no loader |
| Plan on an optimal machine | Tested | 0 changes, 4 skips with reasons |
| Plan on a degraded machine | Tested | 3 changes from `power-saver` |
| Apply + verify | Tested | power profile Confirmed |
| Apply failure reported | Tested | 2 knobs `ServiceUnknown`, reported as failures |
| Partial report | Tested | "1 of 3 optimizations verified" |
| No unmeasured claim | Tested | "Performance impact not measured" |
| Rollback to exact baseline | Tested | restored `power-saver`, not a hardcoded value |
| Rollback touches only applied knobs | Tested | only `power_profile` restored |
| Journal written and cleared | Tested | present while active, gone after |
| Stale journal from previous boot | Unit test | boot id comparison |
| Crash recovery | NOT TESTED | `recover()` is unit-tested; no kill-mid-apply run |
| Battery refusal | Unit test | `refuses_to_raise_power_draw_on_battery` |
| falcond owns the power profile | Unit test | `booster_stands_down_while_falcond_holds_a_profile` |
| Root knobs applied end to end | NOT TESTED | `bigame-daemon` is not installed here, so the Polkit path could not be exercised |

The last row is the significant gap: Polkit authorization, the privileged writes
and their rollback are unit-tested and code-reviewed but have not run against a
live installed daemon.

---

## 4. Gamescope

| Case | Status | Evidence |
|---|---|---|
| Version and flag detection | Tested | 3.16.28, 97 flags parsed |
| Generated args accepted | **Tested** | `gamescope --backend headless <args> -- true` → exit 0 |
| Old `--fsr` rejected | **Tested** | same invocation with old args → `invalid value for --fsr-sharpness`, exit 1 |
| Unsupported flags reported | Unit test | minimal-build fixture |
| Nested launch of a real game | NOT TESTED | see §7 |
| VRR / HDR flags | NOT TESTED — hardware unavailable | flags present, cannot be exercised |

---

## 5. Library and profiles

| Case | Status | Evidence |
|---|---|---|
| Steam detection | Tested | 2 games; 10 runtimes correctly excluded |
| Executable discovery | **Tested** | `PioneerGame.exe`, `DeadByDaylight-Win64-Shipping.exe` |
| Support binaries filtered | Tested | `AntiCheatInstaller.exe`, `EpicWebHelper.exe` dropped |
| Lutris detection | Tested | 5 titles, covers found |
| Heroic detection | NOT TESTED | Heroic present but library caches empty |
| Steam cover art | Tested | hashed subdirectory search; 600x900 and capsule fallback |
| Lutris cover art | Tested | 5 of 5 |
| Grid renders | Tested | screenshot, 4 columns at 1250 px |
| Async cover loading | Tested | no visible stall; scaling fixes column count |
| Orphan profile warning | Tested | the two legacy title-keyed profiles flagged |
| Create / edit / delete profile | NOT TESTED | requires the installed daemon |

---

## 6. Network

| Case | Status | Evidence |
|---|---|---|
| Default-route interface among 42 | **Tested** | `enp7s0`, never a veth/bridge |
| Link speed, MTU, qdisc | Tested | 1000 Mb/s, 1500, fq_codel |
| DNS benchmark, 6 resolvers | **Tested** | medians 12.8–136.3 ms |
| Median/p95/jitter/loss | Tested | computed and displayed |
| Disclaimer present | Unit test | asserts wording |
| IPv6-only resolver | NOT TESTED | code path exists |
| Wi-Fi power save | NOT TESTED — hardware unavailable | not implemented either |

---

## 7. Not tested at all

* **A game launched through the new pipeline.** The two installed titles are
  online and anti-cheat protected; launching someone's competitive games
  repeatedly on their account was not a reasonable thing to do unasked.
* **The privileged helper in production.** Not installed on this machine.
* **Any performance measurement.** See [09-BENCHMARKS.md](09-BENCHMARKS.md).
* **Report view in a live activation.** Unit-tested and rendered, but the
  screenshot of a real activation could not be taken — synthetic keyboard input
  went to the foreground window rather than the application, and driving the
  user's desktop that way was abandoned rather than retried.
* **Packaging.** `meson` and `PKGBUILD` were edited but not built end to end.
* **Translations.** Strings are wrapped in `i18n()`; `.po` files were not
  regenerated, so new strings are untranslated.

---

## 8. Responsiveness

| Width | Status |
|---|---|
| 819 px | Tested — grid at 2 columns, Home centred |
| 1250 px | Tested — grid at 4 columns |
| 1366×768 | NOT TESTED |
| 1920×1080 | NOT TESTED |
| 2560×1440 | NOT TESTED |
| 4K / HiDPI | NOT TESTED |
| Narrow / collapsed sidebar | NOT TESTED |

The display modes on this machine are 3440×1440 and 2560×1080; the window was
resized rather than the display.

---

## 9. Accessibility

| Item | Status |
|---|---|
| Booster control carries its state in its accessible label | Implemented, not screen-reader tested |
| Card grid keyboard reachable, focus reveals actions | Implemented, manually reasoned |
| Enter/Space activate a focused card | Implemented, not tested |
| Reduced motion honoured | Implemented via `gtk-enable-animations` + media query, not tested |
| State never conveyed by colour alone | Implemented — icon and text accompany every state |
| Contrast | NOT TESTED — no contrast measurement taken |
