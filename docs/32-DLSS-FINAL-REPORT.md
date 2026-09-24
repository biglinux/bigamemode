# 32 — AI Graphics: final report

Branch `feature/ai-graphics-dlss`, from `main` at `2b82012`, 2026-09-24.
Development documentation; the application does not read it. Statuses are
exact: VERIFIED (observed on the reference machine), TESTED (automated),
IMPLEMENTED (written and compiled, not observed end to end), NOT TESTED.

## 1. What was analysed

- **DLSS-5-MANAGER** and **DLSS5oneclick**: code, not READMEs
  ([25](25-DLSS-RESEARCH.md)). Both are built around a leaked NVIDIA build
  (`nvngx_dlssnr.dll` + a closed ReShade add-on) and both say so. Their
  engineering around it — records of placed files, PE import parsing,
  proxy-slot ownership, anti-cheat refusal, ini merging, diagnostics — was the
  useful part.
- **Upstream**: OptiScaler (source at v0.9.4, wiki, releases), ReShade,
  RenoDX, NVIDIA DLSS/Streamline licenses, Intel XeSS and AMD FidelityFX
  licenses, VKD3D-Proton, Proton and GE-Proton, Gamescope, lsfg-vk, MangoHud,
  SteamDB's anti-cheat file rules. Where a reference project and upstream
  disagreed, upstream won.
- **Bigamemode itself**: `launcher.rs`, `profiles.rs`, the Video page, the
  dashboard, the wizard. Found and removed: an OptiScaler "staging" that
  copied DLLs over the game's own with no backup, an "AFMF" backend setting
  a RADV option that does not exist, settings nothing read, and a dashboard
  "OptiScaler active" that was true for any `nvngx.dll`.

## 2. Architecture

[28](28-DLSS-ARCHITECTURE.md). A `graphics` subsystem in `bigame-core`
(scan → report → plan → transaction; runtime status; rules; OptiScaler
manager; support report), integrated with `LaunchPlan` through a Harmony
Policy 2.0 that turns off a second upscaler for games BiGame-mode installed
one into. falcond keeps system performance; AI Graphics keeps what happens
inside the game. No root anywhere: game folders and every record are the
user's.

## 3. Taken from DLSS-5-MANAGER (reimplemented)

Backup-once with a record of originals; install records beside the state;
first-free-slot thinking (reduced here to: `dxgi.dll` or nothing, with the
owner named).

## 4. Taken from DLSS5oneclick (reimplemented)

PE import-table reading with positioned reads; marker/records per placed
file (here: manifest with hashes); section-scoped ini edits; anti-cheat
refusal by markers and executable names; log-based diagnostics; a report
zip; a dry-run plan.

## 5. Deliberately rejected

Everything in the leaked DLSS 5 path; NVIDIA-check circumventions (a module
named `nvngx.dll`, an in-memory `NvAPI_GPU_GetArchInfo` patch); bundling any
third-party binary; unverified downloads and self-update; name-based
cleanup at uninstall; "`nvngx.dll` means DLSS"; API guesses from proxy DLL
names; replacing Streamline or NVIDIA DLLs; an anti-cheat override.

## 6–8. Files

**New (27):** `bigame-core/src/graphics/{mod,pe,scan,report,rules,plan,
config,optiscaler,manifest,transaction,runtime,support,text}.rs`,
`bigame-core/src/game_settings.rs`, `bigame-ui/src/views/ai_graphics.rs`,
six examples (`pe_dump`, `graphics_scan`, `graphics_plan`, `graphics_apply`,
`graphics_status`, `optiscaler_fetch`), docs 25–32.

**Modified (21):** `launcher.rs` (Harmony 2.0; staging and AFMF removed),
`models/mod.rs`, `fg.rs`, `hardware.rs`, `lib.rs`, `Cargo.toml`/`Cargo.lock`
(`sha2`, `tempfile` dev), `bench_native_report.rs` (`--vary`), UI: `profile_wizard`,
`profiles`, `game_card`, `home`, `diagnostics`, `video`, `dashboard`,
`fg_controls`, `views/mod`; `locale/extract-strings.py`, `POTFILES.in`,
`bigame-mode.pot`, `pt_BR.po`.

**Removed:** `stage_optiscaler_dlls`, `maybe_stage_optiscaler`,
`resolve_optiscaler_source`; `FrameGenBackend::{OptiScaler, Afmf}` and six
settings fields; dashboard OptiScaler/AFMF detection and the frame-gen
"conflict" badge; the wizard's CPU governor step
([29](29-DLSS-IMPLEMENTATION.md)).

`git diff --stat main...HEAD` (without benchmark data): 48 files, +10 335
−2 010. With the raw benchmark data: 95 files.

## 9. Compatibility rules

[27](27-GRAPHICS-COMPATIBILITY-MATRIX.md), as code in `rules.rs`: never two
upscalers or two frame generators in series; injection blocked with
anti-cheat; each verdict labelled *tested here* / *upstream* / *principle*;
unknown pairs are Unknown.

## 10. Security decisions

- Downloads: HTTPS only, redirects to HTTPS only, pinned SHA-256 (or GitHub's
  published digest for a newer stable release), size-capped, `curl` and
  `bsdtar` with argument vectors — never a shell line. Nothing downloaded is
  executed; the archive listing is checked before extraction (no absolute
  paths, `..`, symlinks, hard links, devices) and the result after.
- Files: every target checked as a plain relative path inside the game
  folder with no symlink on the way, re-checked before the rename; atomic
  place (temp + rename); backups verified by hash before anything changes;
  journal before the first change; rollback on any failure; interrupted
  applies recovered at next start (`recover`); removal by manifest + hash
  only.
- No root: game folders, cache, state and settings are all the user's.
- The support report masks home, user and host in every file and reads no
  environment, credentials or Steam configuration.
- Anti-cheat: no injection, no override.

## 11. Licensing decisions

[26](26-DLSS-LICENSE-AUDIT.md). BiGame-mode ships no third-party graphics
binary. OptiScaler (GPL-3.0) is fetched from its own release on request;
its bundled FSR (FidelityFX v2) and XeSS (Intel) DLLs are placed only when
the configuration needs them; the Agility SDK copy (Windows-only license)
and AMD's `amdxcffx64.dll` are never placed; NVIDIA DLLs are never fetched,
placed or replaced; ReShade's binaries are not fetched (its site asks that
users be sent there); RenoDX is detected, not installed; nothing from the
leaked DLSS 5 path exists in the project.

## 12–13. Tests and real games

[30](30-DLSS-TESTS.md). 511 automated tests (97 for AI Graphics), clippy
pedantic 0 warnings. **Shadow of the Tomb Raider** end to end through the UI
in pt_BR: plan → Apply → loaded by the game with no `WINEDLLOVERRIDES` →
`init successful for fsr31` → Home *Ativo (FSR)* → Restore refused while
running → folder byte-identical after Restore (170 files hashed). Cyberpunk
2077, Rise of the Tomb Raider, Tomb Raider 2013 analysed and planned, not
installed. NVIDIA/Intel GPUs, anti-cheat titles live, ReShade/RenoDX/frame
generation: NOT TESTED.

## 14–15. Benchmarks, before/after

[31](31-DLSS-BENCHMARKS.md). SotTR 3440×1440 High, three runs each, spread
0.2 %:

| | avg fps | vs TAA |
|---|---|---|
| the game's TAA (before) | 89.8 | — |
| the game's own XeSS Quality | 94.2 | +4.9 % |
| **OptiScaler FSR from XeSS Quality (after)** | **98.8** | **+10.1 %** |

1 % lows unchanged; 0.1 % lows too scattered to call. Rendered frames only.
Whether the FSR 4 model or FSR 3 ran is not proven by the log; the UI says
"FSR".

## 16. Known limitations

- The visual-quality comparison is inconclusive (captures not frame-aligned).
- FSR 4 vs FSR 3 cannot be told from OptiScaler's log; only its overlay says.
- Only `dxgi.dll` is used as the slot; a game whose `dxgi.dll` belongs to
  another tool gets an explanation, not an install.
- "Latest stable" OptiScaler is parsed but the UI pins the tested release;
  there is no update prompt yet.
- Steam's own file verification will report the placed files as extra /
  changed; Repair puts back what it removes.
- The install state is per machine; a profile's `[ai_graphics]` choice is
  portable, the installed files are not (by design).

## 17. Experimental

DLSS-input takeover on AMD/Intel (spoofing + fakenvapi); OptiScaler's frame
generation (`OptiFG` → FSR-FG), off unless chosen with *Allow experimental*;
RenoDX/ReShade combinations (rules only).

## 18. Next steps

1. Read OptiScaler's overlay (or its watermark) to settle FSR 4 vs FSR 3, and
   say so in the UI.
2. A frame-aligned image comparison (photo mode or a scripted camera).
3. Cyberpunk 2077 measured with its own benchmark (parser already exists).
4. An update prompt for OptiScaler (*Update · Skip · Keep this version*) on
   top of the existing pin and API parsing.
5. ReShade/RenoDX as an explicit, user-fetched step once a Linux-supported
   path exists upstream.
6. Learned results: record each game's measured outcome locally (the
   benchmark history file already exists) and let it move a plan from
   Compatible to Recommended.
