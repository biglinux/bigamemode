# UX restructure — test report

Run on 2026-09-26 on the reference desktop (AMD Ryzen 7 5700G, Radeon RX
9060 XT + Radeon Vega, BigLinux, KDE Plasma on Wayland, kernel 7.2.7,
falcond 2.0.2 running, no game running). Commands as run, from
`bigame-engine/`.

## Automated

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo build --workspace` | ok |
| `cargo test --workspace` | 568 + 17 + 26 passed, 0 failed (bigame-core, bigame-daemon, bigame-ui) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean (pedantic lints, no allows added beyond the documented ones) |
| `python3 locale/extract-strings.py --check` | `bigame-mode.pot` up to date (1 189 strings) |
| `msgfmt --check --statistics locale/pt_BR.po` | 1 187 translated, 1 untranslated (the empty header entry) |
| `tests/daemon-authorization.sh` | PASSED — every privileged request refused with Polkit unreachable |

New tests: `overview.rs` (10: feature states, attention, headline, the
Proton profile, scheduler states, unsupported V-Cache, frame generation
without the DLL, Turbo/falcond from the unit, the power profile),
`settings.rs` (4: first run, legacy file, round trips, unparsable file),
`window.rs` (2: page migration, removed categories), `widgets/status.rs`
(2: every state has a label, icon and class; neutral states).

## The application, on the real desktop

The debug binary was run as an X11 client with a scratch
`XDG_CONFIG_HOME` (no `settings.toml`, so a first run), driven with
`xdotool` under a pointer-window guard, and captured with `spectacle -a`
after checking the active window. The installed instance was stopped
first and restarted afterwards; the real `settings.toml` was compared
with its backup and is byte-identical.

| Check | Seen |
|---|---|
| First run | opens in Gamer + Dark, on Home; the first save writes `theme = "gamer"`, `color_scheme = "dark"` |
| Navigation | Home, Profiles, Tuning, Details, Logs, Settings; no Video, Benchmark or Diagnostics |
| Saved `last_tab = "diagnostics"` (the user's real file) | `migrate_tab` → Details (unit test; the real file was not used) |
| Details, no game, Turbo on | "Ready to play"; chips: Turbo Active, falcond Active, Profile None, Power performance (waiting), Scheduler Off, GPU Radeon RX 9060 XT, Gamescope Off, Upscaling Off, Frame generation Off |
| Telemetry | six cards updating; disk and latency sparklines move |
| Graphics cards | two cards: Radeon Vega (Cezanne) "Available", Radeon RX 9060 XT "Games render here", each with load bar, clock, VRAM bar, temperature (green), power, `amdgpu · Mesa 26.2.2` |
| Performance rows | Turbo Active; falcond Active "Running · idle, no game" with 10 profiles loaded, the desktop set and the evidence line in its body; Power profile "performance now; performance when a game starts"; CPU scheduler Off "None asked for: the kernel's default scheduler", body with what it is, loaded now, managed by, the installed list; 3D V-Cache **Not supported** (neutral chip), body names the CPU and says nothing is to be done |
| Video pipeline | "No game running" card, the strip with Game/Proton/Display waiting and the stages off, six rows all Off / "Not configured", AI Graphics "Waiting for a game" |
| Problems | "Nothing needs attention"; falcond features (Information), 3D V-Cache (Hardware), Resizable BAR (Information); "9 checks passed" collapsed last; copy-all button |
| Network, Background load, Support report | as before the change |
| Tuning | System performance (performance mode on, scheduler expander, 3D V-Cache **Not supported** row, falcond's settings expander), Display and Gamescope (version 3.16.28, switch off), Upscaling and sharpening (Wine FSR off, quality insensitive, vkBasalt expander, AI Graphics note), Frame generation (global switch, `Lossless.dll`, game combo, per-game controls insensitive), Overlay (MangoHud installed · Per game), Advanced collapsed |
| Profiles | the grid; header: Create with Wizard, rescan, import, new; hover reveals Optimize + ⋮; the ⋮ menu of a Lutris game without a profile: Launch (Turbo), Create with Wizard, Create profile, Measure the difference |
| Settings | Appearance toggles; Default → the Adwaita look at once, file loses `theme`; Light → `color_scheme = "light"`; Gamer + Dark restored |
| Themes | Gamer dark, Gamer light, Default dark, Default light all rendered on Home and Details; chips, cards and the pipeline strip legible in each |
| Window sizes | 800×600, 1024×768, 1280×800, 1920×1080: no cut text, no horizontal scroll; telemetry and GPU cards reflow to one column at 800 px, the pipeline strip wraps to two rows |
| Journal / stderr | no GTK warning or critical from the new pages (one from the Tuning example command line was found and fixed: `<game>` was parsed as markup) |
| CPU while Details is focused | 2–3.6 % of one core (debug build, cairo renderer), from `top` over 15 s; `bigame-ui` is listed in Background load at 19 % during the screenshot session, which included the capture tooling |

## Not tested on real hardware

- A running game on the new pages (Gamescope, Wine FSR, vkBasalt, lsfg-vk
  and MangoHud detected or not detected; the AI Graphics row Active; the
  GPU card "Renders <game>"): no game was started for this run. The
  decisions are unit-tested in `overview.rs`; the detection code they read
  (`running::in_game`, `graphics::status_running`, the environment read)
  is unchanged from before.
- Turbo off, falcond failed, falcond silent, a missing helper: unit-tested
  headlines and states; not reproduced on the desktop.
- 3D V-Cache present, sched-ext missing or `scx_loader` down, Gamescope /
  vkBasalt / lsfg-vk not installed: this machine has none of these states
  (no X3D, all installed). The rows and the Tuning *missing* rows are
  built from the same capabilities the old pages used.
- NVIDIA, Intel and hybrid machines.
- Launch (Turbo) and Create with Wizard from the card menu were shown, not
  pressed (a launch starts a game; the wizard saves through Polkit).
