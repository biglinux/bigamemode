# AI Graphics — backend architecture

AI Graphics used to be one path: OptiScaler, placed beside the game. It is
now an engine that knows three ways to reach a game, tells three jobs
apart, checks requirements as data, keeps the conflict rules in one table,
places files through one transaction layer, verifies the running game, and
takes measurements from this machine before it recommends anything.

```text
                         AI GRAPHICS
                              │
          ┌───────────────────┼────────────────────┐
          ▼                   ▼                    ▼
       Native             OptiScaler        AMD neural (external)
  the game's own      placed by BiGame-mode   DLSS-NR-on-AMD, the user's
  FSR / XeSS / DLSS,  as dxgi.dll, backed up  own install: detected,
  FSR 4 through       and undone as one       explained, linked —
  Proton's provider   transaction             never placed
          │                   │                    │
          └───────────────────┼────────────────────┘
                              ▼
                     backend::check  →  what is missing, in words
                              ▼
                     rules::problems →  one technology per job
                              ▼
                     plan            →  backend, frame generation, steps
                              ▼
                     transaction     →  OptiScaler only (managed: true)
                              ▼
                     runtime / external / native_runtime → what the game loaded
                              ▼
                     outcomes       →  measured here, read back by the plan
```

## Backends (`bigame-core/src/graphics/backend.rs`)

| | Native | OptiScaler | AMD neural, external |
|---|---|---|---|
| GPUs | any | AMD, NVIDIA, Intel | AMD RDNA 3 / 4 |
| Game | any | 64-bit Windows game (Proton / Wine) | 64-bit Windows game, DirectX 12, ships the FidelityFX API |
| Jobs | upscaling, frame generation | upscaling, frame generation | neural rendering |
| Files placed | none (one Steam launch option at most) | `dxgi.dll`, `OptiScaler.ini`, FidelityFX / XeSS runtimes, backed up | none: `managed: false` |
| Maturity | verified here | verified here | experimental; documented for Windows only |

`Capabilities` is data; `check(backend, &Report)` returns `Available` or
`Unavailable { missing }`, where each `Missing` names the requirement and
what was found, so a page never greys a control out without saying why. GPU
vendor decisions live here and in `report::family()`, not across the tree.

## Three jobs, one owner each

| Job | Owners | Where it is decided |
|---|---|---|
| Upscaling | the game's own; OptiScaler (FSR / XeSS / DLSS output); Gamescope and Wine FSR are turned off when either is on | `plan.backend` |
| Neural rendering | DLSS-NR-on-AMD only, on top of the game's own FSR; never with OptiScaler until a test shows the pair stable | `Analysis.neural` (`external::Status`) |
| Frame generation | the game's own, OptiScaler's (OptiFG), or lsfg-vk — one at most | `plan.frame_generation` |

The pairs are in `rules.rs` with a verdict and its basis (tested here,
upstream, principle). Anti-cheat blocks every injecting technology,
including the external backend, with no override.

## Verification, never assumption

| Claim | Evidence |
|---|---|
| OptiScaler loaded / active / failed | `/proc/<pid>/maps` and `OptiScaler.log` written since the process started |
| FSR 3.1 through OptiScaler | its log (`Fsr4Update: false` or the provider missing) |
| FSR 4 through OptiScaler | never claimed from a log; its overlay is the only proof |
| FSR 4 through Proton, game's own FSR | `FSR4_UPGRADE=1` in the running game's environment **and** `amdxcffx64.dll` mapped (`native_runtime`) |
| DLSS-NR-on-AMD active | its proxy mapped and its log's "loaded into" banner since the process started |
| The GPU the game renders on | DRM fdinfo of the render node it submitted work to |

## What the plan reads

- The report (files, running process, GPUs, Proton prefix, game list).
- The launch context: Gamescope, Wine FSR, lsfg-vk, MangoHud, the
  OptiScaler version policy, whether `FSR4_UPGRADE=1` is set.
- Measurements on this machine (`outcomes.rs`): rendered frames only;
  presented frames (frame generation) are recorded apart and never compared
  with rendered ones. OptiScaler is Recommended only when measured faster
  with the 1 % low no worse; the game's own upscaler otherwise.

## Manifest and external installs

`Source.backend` records which backend placed the files (older manifests
read as OptiScaler's); `Manifest.managed` is true for everything BiGame-mode
placed. The external backend gets no manifest: nothing removes files
BiGame-mode does not manage.

## Examples (`bigame-core/examples/`)

`graphics_plan`, `graphics_capabilities`, `graphics_diagnose`,
`graphics_native` (the FSR 4 launch option), `graphics_apply`,
`graphics_status`, `bench_native_report --record-graphics`.
