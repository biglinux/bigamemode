# Shadow of the Tomb Raider — sched-ext schedulers through falcond (2026-09-25)

Lab laptop (i7-7700HQ 4C/8T, GTX 1050 Ti Mobile renders), lowest preset,
1920×1080, DX12, XeSS Performance → OptiScaler FSR 3.1, Turbo on. The
scheduler is set in the game's falcond profile (`scx_sched`, mode `gaming`),
saved through BiGame-mode's helper (administrator password each time);
falcond loads it through scx_loader when the game starts and unloads it when
the game exits. Checked during the runs: `/sys/kernel/sched_ext/state`
`enabled`, `root/ops` `lavd_1.1.3…` in a lavd run; `disabled` again after the
game, power profile balanced and governor schedutil restored.

Order A B C C B A (runs 01–06), then A B C (runs 07–09). Run 01's MangoHud log
was not captured (the loading-screen detection timed out while a build ran
beside it); its game result is valid.

| Run | Scheduler | Rendered frames (game) | Game avg | MangoHud avg | 1 % low | p99 |
|---|---|---|---|---|---|---|
| 01 | none | 5692 | 37 | — | — | — |
| 02 | lavd | 6245 | 40 | 44.5 | 19.4 | 40.8 ms |
| 03 | bpfland | 5899 | 38 | 41.1 | 13.5 | 52.2 ms |
| 04 | bpfland | 6191 | 40 | 44.0 | 21.8 | 35.2 ms |
| 05 | lavd | 5874 | 38 | 42.7 | 17.1 | 43.8 ms |
| 06 | none | 5854 | 38 | 41.3 | 14.8 | 49.1 ms |
| 07 | none | 6042 | 39 | 42.8 | 18.1 | 42.3 ms |
| 08 | lavd | 6172 | 40 | 43.6 | 16.1 | 45.5 ms |
| 09 | bpfland | 6319 | 41 | 45.5 | 21.7 | 33.9 ms |

Rendered frames: none 5863 ± 175, lavd 6097 ± 196 (+4.0 %), bpfland 6136 ±
215 (+4.7 %). Welch's t 1.5 and 1.7 against a 95 % critical value near 2.8:
**no measurable difference**. Every arm got faster through the evening
(none: 37 → 39), which is drift of the size of the differences. The profile
was left with `scx_sched = none`.
