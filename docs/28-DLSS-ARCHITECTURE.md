# 28 — AI Graphics architecture

How upscaling, frame generation and the files a game needs for them fit into
BiGame-mode. Development documentation; the application does not read it.

## Responsibilities

| | Owns | Does not touch |
|---|---|---|
| **falcond** (Turbo) | CPU power profile, scheduler, V-Cache, idle inhibit — system performance while a game runs | anything inside a game's folder |
| **AI Graphics** (`bigame-core/src/graphics/`) | which upscaler and frame generator a game uses, and every file BiGame-mode places in its folder for that | CPU, scheduler, governor, power |
| **LaunchPlan** (`launcher.rs`) | how BiGame-mode starts a game: Gamescope, environment | game files |

AI Graphics is not another performance daemon: it runs when the user opens
it, and at launch only to ask "did BiGame-mode install something in this
game?".

## Flow

```text
Profile / game card ─► AI Graphics page
                         │
        scan ─► report ─► plan  (dry run: steps, in-game choice, files, disables)
          │       │        │
          │       │        ├─ Apply ─► fetch (pinned, SHA-256) ─► payload ─► transaction
          │       │        │                                                  (backup → journal →
          │       │        │                                                   place → verify → commit)
          │       │        ├─ Repair ─► missing files back from the cache
          │       │        └─ Restore ─► manifest-driven removal, originals back
          │       │
          │       └─ running game (VKD3D-Proton / DXVK mapped, render card) turns guesses into facts
          │
Launch ─► launch_disables(process) ─► LaunchPlan harmony: no second upscaler for that launch
Running ─► runtime::status  (maps + OptiScaler.log since the process started)
          ─► Home card · Diagnostics · AI Graphics page
```

## Modules

| Module | Role |
|---|---|
| `graphics/pe.rs` | PE reader: machine, import and delay-load tables (read in place, a few KB whatever the file size), version from `VS_VERSIONINFO` through the resource directory |
| `graphics/scan.rs` | bounded folder walk (depth 5, 40 000 entries, no symlinks): executable, DLSS/DLSS-G/DLSS-RR, Streamline, XeSS/XeSS-FG/XeLL, FSR, mod configs, proxy-slot DLLs with their owner from contents, anti-cheat markers |
| `graphics/report.rs` | report with confidence per value (fact / detected / likely / assumed); GPU name from the PCI ID database, RDNA generation, userspace driver, render card |
| `graphics/rules.rs` | compatibility matrix as code ([27](27-GRAPHICS-COMPATIBILITY-MATRIX.md)) |
| `graphics/plan.rs` | dry-run planner, five standings (Recommended / Compatible / Experimental / Not recommended / Blocked) |
| `graphics/optiscaler.rs` | release pinning, API "latest stable" parsing, download with `curl`, listing check and `bsdtar` extraction, ini editing, payload, log reading |
| `graphics/manifest.rs` | the per-game record: entries with hashes, backups with hashes, created folders, run-time files, the game's process |
| `graphics/transaction.rs` | apply / rollback / recover / remove / verify / repair |
| `graphics/runtime.rs` | real status from `/proc/<pid>/maps` and a log written since the process started |
| `graphics/support.rs` | redacted support zip |
| `graphics/config.rs` | the portable per-game choice (`AiGraphicsConfig`) |
| `game_settings.rs` | BiGame-mode's own per-game settings file, apart from falcond's profile |
| `graphics/mod.rs` | facade: `analyze`, `install`, `remove`, `repair`, `status`, `status_running`, `installed`, `launch_disables`, `target_for_process` |

UI: `bigame-ui/src/views/ai_graphics.rs` (the page), the profile wizard's AI
Graphics step, the game-card menu, the Home card's status line, the
Diagnostics section.

## Where things live

| What | Where | Why there |
|---|---|---|
| Per-game choice (`[ai_graphics]`) | `~/.config/bigame-mode/games/<process>.toml` | user-owned, portable, not in falcond's root-owned profile (falcond ignores the fields, and the profile migration would drop them) |
| Downloaded releases | `~/.cache/bigame-mode/graphics/optiscaler/<version>/` (+ `release.json`: URL, tag, SHA-256, date, license) | one copy for every game; can be deleted and fetched again |
| Manifests, backups, staging, last logs | `~/.local/state/bigame-mode/graphics/<steam-appid|exe-hash>/` | the record of what changed, which must survive a cache clean |
| Placed files | the game's executable folder | where the game loads them from |

No root is involved: game folders belong to the user, and so does
everything above.

## Decisions

- **Only `dxgi.dll`.** Proton already loads `dxgi` natively (it sets it for
  DXVK), and the application folder is searched first — verified: OptiScaler
  loaded as `dxgi.dll` with no `WINEDLLOVERRIDES`. Other slots would need an
  override, which for Steam games means launch options Steam must be closed
  to change. When `dxgi.dll` belongs to another tool the plan stops and says
  so; it never overwrites.
- **Update = remove, then apply.** The original of every file is always the
  file from before BiGame-mode, never a previous BiGame-mode payload.
- **FSR 4 is never claimed from configuration.** The log proves which backend
  was created; whether the FSR 4 model runs (and not FSR 3) is shown only by
  OptiScaler's overlay, and the UI says "FSR".
- **Frame generation is never automatic** — it raises presented, not rendered,
  frames and adds latency — and OptiScaler's is Experimental.
- **No override for anti-cheat.** Detected anti-cheat blocks injection in
  every mode; the game's own upscaler is still offered, as a menu setting.
- **Local only.** Nothing is sent anywhere. The one network access is the
  user-initiated download from the project's GitHub release.
