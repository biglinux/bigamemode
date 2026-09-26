# AI Graphics — final audit of the backend work (2026-09-26)

Branch `feature/ai-graphics-backends`, reference desktop (Ryzen 7 5700G,
Vega iGPU idle, RX 9060 XT, Mesa 26.2.2, Proton Experimental 11.0-100).
Every claim below names its evidence; what was not proven is listed as
such.

## Acceptance criteria

| Criterion | State | Evidence |
|---|---|---|
| Existing qualities kept: analysis, dry-run, SHA-256, cache, transaction, rollback, recovery, Repair, Restore, anti-cheat block, runtime detection, no downloads at launch | kept | the modules are unchanged in behaviour; 551 core tests pass; Cyberpunk apply → status Active → Restore put `amd_fidelityfx_dx12.dll` back with its original hash |
| Extensible backends with centralized capabilities | done | `backend.rs`: `Backend`, `Capabilities`, `check()`; no vendor branching outside it, `report::family()` and `rules.rs` |
| Upscaling, neural rendering and frame generation told apart | done | `plan.backend`, `plan.frame_generation`, `Analysis.neural` |
| AMD first, fresh hardware detection | done | families from the PCI name at every analysis; RDNA 4 verified |
| OptiScaler + FSR the main AMD backend; FSR 4 requirements from current docs | done | `AI_GRAPHICS_AMD_IMPLEMENTATION.md`; SOTTR +10.1 % |
| FSR 4 never claimed without evidence | done | "FSR 4 expected" on the page; Home says "FSR 4 (the game's own, Proton's provider loaded)" only with the provider mapped and the variable in the environment; OptiScaler's FSR stays "FSR" |
| `FSR4_UPGRADE=1` only if still valid, explained once | done | verified against Proton Experimental 11.0's `amdxc64.dll` (`getenv`), provider mapped only with it, `Replaced FSR3 with FSR4!` logged |
| DLSS-NR-on-AMD external only: detect, explain, link, Detect again; never download/install | done | `external.rs`; no network code, no transaction, `managed: false`; UI has no Install |
| POC under Proton with a legitimately provided binary | not possible | none provided; HIP runtime absent under Proton; result "not currently compatible" (`AMD_NEURAL_RENDERING_POC.md`) |
| OptiScaler and the neural backend exclusive; conflict matrix | done | `rules.rs`: Conflict with both OptiScaler techs, Blocked with anti-cheat, Experimental with native FSR, Unknown with ReShade |
| Better executable / API detection with evidence | done | FidelityFX API and `d3d12core.dll` as evidence; the running process is the fact |
| No dgVoodoo, no 32-bit host64 | done | neither exists |
| NVIDIA and Intel not broken | done | their tests pass; laptop paths unchanged |
| Hybrid GPU by DRM fdinfo | kept | Vega + RX: the RX chosen; "confirmed when the game runs" |
| Manifest `backend` and `managed` | done | old manifests read as OptiScaler / managed (test) |
| Version pinning / Go back | kept | unchanged |
| Game list expansion | done | `bad_optiscaler`, `proxy`, `verified_gpu`, `verified_proton`, `notes`; SOTTR and Cyberpunk entries |
| Diagnose | done | `diagnose.rs`, the page's group, `diagnose.txt` in the report |
| Support report additions with masking | done | system, proton, conflicts, neural, loaded modules, the component's log; the existing redaction applies |
| Simple UI, libadwaita, missing requirements shown, badges by evidence | done | Current / Recommended / Neural rendering / Technical details / Diagnose; screenshots taken in Default light and Gamer dark |
| Rendered vs presented | done | `outcomes::Frames`; presented never compared with rendered |
| A/B benchmarks for new recommendations | done | Cyberpunk session, three arms, two runs each; recorded in the local outcomes and read by the plan |
| Regressions | clean | fmt, clippy 0 warnings, 586 tests, daemon authorization, translations |
| Security: argument vectors, unprivileged | done | no `sh -c` in the new modules; nothing new in the daemon |
| Anti-cheat absolute | done | Blocked for every injecting technology, including the external one; no override |
| No third-party bundling | done | PKGBUILD unchanged; `AI_GRAPHICS_LICENSE_AUDIT.md` |
| Documentation per stage | done | source, license, backend architecture, AMD implementation, neural POC, test matrix, this audit |
| gettext + pt_BR | done | template up to date, 1111 messages translated |

## Not proven, and said so

- Which FSR model rendered in the `native_fsr4` arm: Proton logged the
  replacement and the provider was mapped, but no image was compared.
- The AMD neural component under Proton: never ran here.
- Hardware other than RDNA 4 and the lab laptop's GTX: unit fixtures only.
- The lows in the Cyberpunk session (two runs per arm) could not be
  compared; the averages could.

## Package

Built with `makepkg` from the committed branch (a scratch copy of the
PKGBUILD pointing at the local repository, under `~/.cache`, since `/tmp`
is mounted `noexec`) and installed with `pacman -U`; the daemon restarted
and serves the system bus; the application opened Cyberpunk 2077's AI
Graphics page in Portuguese: Current, the recommendation with "Adicionar a
opção de inicialização", Renderização neural with its missing items,
Detalhes técnicos and Diagnosticar, in the Default and Gamer themes.

## Found during the audit

- The four new modules were missing from `locale/POTFILES.in`, so their
  strings had no Portuguese until this audit; the template check reports
  only listed files. Added, translated (1111 messages), rebuilt.
- In the Gamer theme the plan's title, a sentence, was ellipsized; it now
  wraps.
