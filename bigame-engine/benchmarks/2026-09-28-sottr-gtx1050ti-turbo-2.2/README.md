# Shadow of the Tomb Raider — Turbo off versus on, BiGame-mode 2.2 (2026-09-28)

Lab laptop: Core i7-7700HQ, Intel HD 630 (drives the panel), GeForce GTX 1050
Ti Mobile (renders; NVIDIA 580.178.04), KDE Plasma Wayland, on AC. The game's
built-in benchmark, DX12 (VKD3D-Proton, Proton Experimental), 1920×1080
borderless fullscreen, preset Low, Intel XeSS Quality taken over by OptiScaler
0.9.4 (FSR 3.1), VSync off, MangoHud loaded, no frame generation, no Wine FSR
(Steam restarted with the session's environment after Turbo was switched off).

`scripts/bench-game.sh turbo_off turbo_on`: one launch, a discarded warm-up
pass, then 3 passes per arm with the order rotated, Turbo switched through the
helper and Polkit as the Home button does. What each arm had in force,
recorded with every run (`state.txt`):

| Arm | Power profile | Governor | sched-ext | falcond profile |
|---|---|---|---|---|
| `turbo_off` | balanced | schedutil | none | — (falcond stopped) |
| `turbo_on` | performance | performance | lavd | `SOTTR.exe` |

## Results (bench_native_report, the game's own frame times)

| Arm | Avg FPS | 1 % low | 0.1 % low | p99 | GPU clock | GPU temp | GPU busy |
|---|---:|---:|---:|---:|---:|---:|---:|
| `turbo_off` | 24.1 | 17.0 | 10.1 | 51.4 ms | 1661 MHz | 91.5 °C | 100 % |
| `turbo_on` | 24.1 | 17.3 | 10.4 | 50.7 ms | 1659 MHz | 91.9 °C | 100 % |

No difference above normal variation in any metric (average 0.0 %, 1 % low
+1.8 % within a 2.0 % spread, 0.1 % low +3.1 % within 3.6 %). The GPU was busy
100 % of every pass: what Turbo changes (CPU frequency policy, the
scheduler, the power profile) cannot move a game the GPU limits. Turbo was
really in force and falcond applied the game's own profile; it just has
nothing to gain here.

Caveats: the CPU package ran at 91–97 °C with the kernel counting thermal
throttling (package throttle count 8591 before the session), and the GPU at
91–92 °C; both arms alike, alternated. Frame times, GPU samples and the
game's logs are not in the repository.
