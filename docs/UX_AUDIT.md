# UX audit — control by control

Every interactive control in the GTK4/libadwaita UI was traced to what it
calls, whether the result is read back from the system, where it is kept, and
what it shows when the feature is missing. This page records what was wrong
and what was done; controls that were correct are not listed.

## Controls that claimed something that did not happen

| Control | What it did | Now |
|---|---|---|
| Profiles → "Activate Profile" | showed the toast "Profile activated", nothing else | removed — falcond applies a game's profile when its process starts |
| Tray → "▶ ‹profile›" items | wrote a log line | removed, same reason |
| Profile review → "Use general optimization" | the same as "Not now" | removed |
| Profile → "Use Gamescope" (Automatic / Always / Never) | saved, never read: Never did not stop wrapping | honoured by the launcher; Automatic follows the Video switch |
| Details → "Launch (Turbo)" on a Steam game | tooltip promised BiGame-mode's video settings; the Steam client starts the game in its own process tree, so none applied | tooltip says what happens: the profile applies, video settings reach Steam games only through Steam's launch options |
| Video → "Integer Scaling" | emitted `-F pixel`, Gamescope's pixel-art filter | emits `-S integer` where the installed Gamescope has it |
| Video → Wine FSR / vkBasalt off | removed the environment.d file, left the variable in the running session until logout | the session is updated too (`UnsetAndSetEnvironment`) and read back |
| Video → vkBasalt | offered even when not installed; described as a feature, not as the visual filter it is | shown only where the layer is installed; says it is a look, not a speed-up |
| Profile / Tuning / Video → frame generation | wrote a format lsfg-vk 1.0.0 does not read; "off" invalidated the whole file | see [LOSSLESS_SCALING_VALIDATION.md](LOSSLESS_SCALING_VALIDATION.md) |
| Details → frame generation "Active (Generating Frames)" | shown when the layer was mapped in the game, which it is in every Vulkan process | "On for this game" only with the layer loaded **and** an entry for the game |
| Profile wizard → V-Cache "Cache mode" | saved `freq` | saves `cache` |
| Tuning → "falcond scan interval (ms)" | changed a value in memory, never saved; duplicated "Poll Interval (ms)" | removed; its explanation moved to the control that saves |
| Menu → Toggle Dark Mode | never persisted | the choice is kept across restarts; until one is made, the desktop's is followed |
| Profiles → MangoHud (a Steam game with Steam's default settings) | "Steam has no entry for app 750920", yet saved "on": the page showed MangoHud on and the game never got it | the game's block is created in `localconfig.vdf`; the choice is saved only after the launch options read back (checked: `MANGOHUD=1` in the game's environment, `libMangoHud.so` mapped) |
| Tuning → Frame Generation "tune in real time" | lsfg-vk does not start or stop generating for a game already running | says that on and off take effect at the next start; Home warns when the file changed after the game started |
| Video → Gamescope for a game that prefers Wayland | the game was ended at start by Gamescope's WSI layer | `--expose-wayland` where the build has it |
| Video → vkBasalt with Gamescope | ran on Gamescope's own output (after its FSR) and never in the game: Gamescope removes `ENABLE_VKBASALT` from its child | off for Gamescope, on for the game |
| Menu → Toggle Dark Mode | a two-state toggle, no way back to the desktop's choice | Settings → Appearance: Default or Gamer, System, Light or Dark |
| Tuning and profile editor → V-Cache | a disabled group (Tuning) and a live combo (profile editor) on a CPU without 3D V-Cache | hidden; Diagnostics says the CPU has none |
| Settings → "Fix old profiles" | offered to "clean" profiles the current version had just saved, dropping their frame generation settings | only profiles an older version wrote are offered, and their current fields are kept |

## Readings that described the wrong thing

| Where | Was | Now |
|---|---|---|
| Details → GPU Freq | `card1/device/pp_dpm_sclk`: amdgpu-only, and `card1` is the Intel GPU on the lab laptop | the GPU card follows the card the running game has open (or the expected one): clock and load per driver, NVIDIA through NVML |
| Details → GPU Temp | the first `hwmon*/temp1_input` on the machine, any sensor | that card's own sensor, "Not reported by the driver" where it has none, with the reason the clock is held down (power limit, temperature) |
| Home → GPU tile | the static "games' GPU" guess | the card the running game uses |
| Home → game card | no GPU at all | "… on GeForce GTX 1050 Ti Mobile" |
| Details → (new) Graphics cards | — | each card and its role: renders the game, games start here, available, asleep |
| Diagnostics → System health | computed once, when the page was built; stale after a service started | recomputed every time the page is shown |
| Diagnostics → sched-ext | "installed but its service is not running" before scx_loader's first use, though the bus starts it on demand | activatable counts as available |
| Scheduler lists | every `/usr/bin/scx_*`, including `scx_loader` and six schedulers falcond 2.0.2 does not know | the schedulers falcond lists as available and that are installed |
| Diagnostics → (new) CPU temperature | — | the kernel's thermal-throttle counters (1799 events in a morning of benchmarks on the lab laptop) |
| Diagnostics → (new) Hybrid graphics | — | where games render, how they get there, and what a native Steam game needs |
| Home and Details → the running game's GPU | "the non-boot card of several render nodes": the Vega for any Vulkan game on the AMD desktop, since enumeration opens every node | the card the game submitted work to (DRM fdinfo); "Radeon RX 9060 XT" |
| GPU names | "Navi 44 [Radeon RX 9060 XT]", "Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]"; Home's summary "GPU AMD" | "Radeon RX 9060 XT", "Radeon Vega (Cezanne)" everywhere |
| Details → frame generation, Gamescope | keyed on falcond's active profile ("Proton" for a game without its own), and any Gamescope on the machine counted | keyed on the watched game's process; Gamescope only in its tree |
| Home → (new) "In the game" | — | what the game really got: frame generation, Gamescope, MangoHud, vkBasalt, scheduler |
| Turbo report → power profile | "restores it when the game exits" | falcond puts back the profile it saw when the service started, and the report says so |
| Turbo report → CPU governor | "Verified" while power-profiles-daemon's companion had already put it back | the governor is power-profiles-daemon's; the report says so |

## Responsiveness

- Home hashed every installed AI Graphics file on the GTK main thread every
  two seconds (ten in a game); it runs off the main thread now.
- The launcher ran `gamescope --help` twice per launch; once now.
- Measured on the lab laptop: `bigame-ui` idle in the tray with its window
  open uses 0.25 % of one core (about two wake-ups a second on the main
  thread). Its three-hour average earlier in the day was 2.6 %, with the
  Details page open during games; see
  [FINAL_VALIDATION.md](FINAL_VALIDATION.md).

## Still to do

- Details pings the latency target once a second while the page is shown.
- The Optimization Report's details (the Booster's plan) are English only:
  `booster/plan.rs` builds them with `format!`, outside the translation list.
- The Benchmark page runs nothing itself; measuring a Proton game from the
  UI is not possible yet (Steam games have no launch command BiGame-mode can
  wrap).
