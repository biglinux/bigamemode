# 2026-09-23-sottr-ui-overhead — 2026-09-23

## Verdict

- **ui_paused** — NO CHANGE: a 1.1% difference, but Welch's t = 2.38 falls short of the 2.78 needed for 95% confidence at 3.4 degrees of freedom

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| ui_polling | 3 | 115.0 | 114.8 | 114.4 | 115.8 | 0.7% | — |
| ui_paused | 3 | 116.2 | 116.4 | 115.7 | 116.6 | 0.4% | within noise |

## Method

- 3 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `02db452880e054ab`.

## Caveats

- The product's own overhead: the running BiGame-mode UI (installed build, 1-second dashboard poll) either polling as usual or frozen with SIGSTOP; same machine state (rest: performance profile, DPM auto) in both arms, resumed with SIGCONT afterwards.
- Shadow of the Tomb Raider CPU-bound: render scale ("Modificador de resolucao") at its minimum, 3440x1440 output, GPU ~84% busy. Restored afterwards and verified by diffing a following run's Settings block against 2026-09-23-sottr-power-clean.
- With falcond's 'Proton' profile active, the poll forked about 32 processes a second (pgrep x4 plus timeout+grep per matching pid, 7 pids), out of 55/s system-wide.
- Result: +1.1% with the UI frozen, every frozen run above every polling run, but Welch's t = 2.38 against 2.78 at three runs per arm -- reported as a suggestion, not as a measured gain.

## Raw runs

- `ui_paused`: 116.4, 116.6, 115.7
- `ui_polling`: 114.8, 115.8, 114.4
