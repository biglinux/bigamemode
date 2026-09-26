# Cyberpunk 2077: the game's own FSR, FSR 4 through Proton, OptiScaler — reference desktop

The first AI Graphics session on a game that ships AMD's FidelityFX API
(`amd_fidelityfx_dx12.dll` 1.0.1.41314, FSR 3.1), the path Proton upgrades to
FSR 4. Cyberpunk 2077 2.3, DirectX 12, ray tracing Ultra (custom), 3440×1440,
upscaling preset Auto, the game's own benchmark (`benchmarkResults/*/summary.json`
and `frames.csv`, one pass per run), RX 9060 XT (RDNA 4, Mesa 26.2.2), Proton
Experimental 11.0, MangoHud loaded, Turbo on. Two passes per arm; the first
pass of the launch was the warm-up.

| Arm | What ran | Evidence |
|---|---|---|
| `native_fsr31` | the game's FSR (menu), no launch option | `amdxcffx64.dll` never mapped in `/proc/<pid>/maps` |
| `native_fsr4` | the same with `FSR4_UPGRADE=1 %command%` in the Steam launch options | `amdxcffx64.dll` mapped; Wine's `amdxc` channel logged `Replaced FSR3 with FSR4!` |
| `optiscaler_fsr` | OptiScaler 0.9.4 placed by AI Graphics (`dxgi.dll`), the game's XeSS input → FSR output, `Fsr4Update: true`, `amdxcffx64.dll` loaded by OptiScaler | `OptiScaler.log` (kept here) |

| Arm | avg fps | 1 % low | p99 frame time |
|---|---:|---:|---:|
| native_fsr31 | 38.5 · 38.3 | 31.1 · 28.2 | 31.4 ms |
| native_fsr4 | 38.3 · 38.2 | 30.5 · 26.1 | 31.8 ms |
| optiscaler_fsr | 36.0 · 36.0 | 28.5 · 29.3 | 33.5 ms |

Verdicts (`report.md`, against `native_fsr31`): FSR 4 through Proton **no
difference** (−0.6 %, Welch's t 1.90 with two runs per arm); OptiScaler
**measurably slower**, −6.4 % (t 24.7). The lows vary too much between two
runs to compare.

What this shows:

- **FSR 4 through Proton costs nothing measurable** on this card at this
  workload, and the game's menu is the only thing the user touches. The
  proof is the provider mapped in the running game plus the variable in its
  environment — what BiGame-mode's Home and Diagnose now read — not a log
  line: Proton's `amdxc` channel is off by default.
- **OptiScaler brings nothing here.** A game that ships the FidelityFX API
  already reaches FSR 4 through Proton; OptiScaler adds a proxy, its own
  copy of the runtime and a second pass of its input's inputs. The
  OptiScaler arm fed the game's XeSS to FSR (Auto preset in both, so the
  render resolution may differ) — the fairest input it offers when the
  game's own FSR is what it replaces.
- The planner reads these arms from the local measurements
  (`bench_native_report --record-graphics`): for this game it keeps the
  game's own FSR as Recommended and says no upscaler reaches 60 from 39
  rendered.

Not measured: which FSR model rendered in `native_fsr4` (the overlay is the
only proof; Proton logged the replacement), latency, and presented frames.
The `UserSettings.json` of the OptiScaler arm differs in `FSR3Enabled`,
`XeSSEnabled` and `upscalingType`, which is the arm's own change; the report
was produced with those keys listed under `--vary`.
