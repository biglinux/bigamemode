# Shadow of the Tomb Raider — Turbo off versus on, Big Game Mode 2.4.0 (2026-10-18)

Reference desktop, same game settings as `2026-10-18-sottr-rx9060xt-cap60-2.4`
(the `pretty` launch, no cap). `scripts/bench-game.sh turbo_off turbo_on`:
one launch, a discarded warm-up pass, then 2 passes per arm in the order
off, on, on, off, Turbo switched through the helper as the Home button does
(falcond stopped versus falcond's `Proton` profile: performance power
profile, no sched-ext scheduler, split-lock and VRAM protection).

| Arm | Avg FPS | 1 % low | 0.1 % low |
|---|---:|---:|---:|
| `turbo_off` | 79.5 · 81.9 | 34.5 · 37.5 | 15.6 · 19.8 |
| `turbo_on` | 80.7 · 80.0 | 35.7 · 37.1 | 17.8 · 20.2 |

- Average: −0.4 %, within the 2.0 % spread of the runs: **no difference**.
- The lows: the runs within an arm disagree too much to compare.

Turbo with 2.4.0 does not cost this game anything; as in earlier sessions on
both machines, the power profile and the scheduler do not move a game the
GPU paces.
