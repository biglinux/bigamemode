# 06 — Gamescope and Tuning

## 1. The question, answered

**Complementary, not mutually exclusive.** They act on different layers:

```
Presentation   Gamescope, FSR/NIS, frame generation, VRR, HDR, FPS cap
Execution      scheduler, V-Cache, governor, EPP, GPU DPM, power profile
```

Nothing in the execution layer changes what Gamescope does, and nothing
Gamescope does changes what the scheduler does. Running both is the normal case.

The real conflicts are *inside* the presentation layer, where several components
implement the same function. Those are enumerated in §4.

## 2. What was broken

`gamescope::Config::to_args()` emitted `--fsr`. Gamescope removed that flag, and
because it shares a prefix with `--fsr-sharpness` the parser does not merely
ignore it — it consumes the next argument and aborts. Measured against the
installed 3.16.28:

```
$ gamescope --backend headless -w 1920 -h 1080 --fsr --fsr-sharpness 5 -- true
gamescope: invalid value for --fsr-sharpness, "--fsr-sharpness" is either
not an integer or is far too large
exit 1

$ gamescope --backend headless -w 2560 -h 1080 -W 3440 -H 1440 \
            -F fsr --fsr-sharpness 3 -r 75 --adaptive-sync --hdr-enabled -f -- true
[gamescope] Info console: gamescope version 3.16.28
exit 0
```

The second line is what the new builder produces for this machine.

Worse than the flag itself: the project contained **two** argument builders, and
`launcher.rs` already built `-F fsr` correctly. Two implementations of the same
thing, one of them broken, with no way to tell which one a given launch used.

## 3. Capability gating

There is now one builder, and its signature makes the mistake unrepresentable:

```rust
pub fn to_args(&self, caps: &GamescopeCaps) -> Args
```

It cannot be called without capabilities. Those come from parsing `--help` on
the binary actually on disk — not from a version-to-feature table, because
distributions patch Gamescope heavily and a version number does not tell you
what a given build accepts.

Anything requested but unsupported is **reported, not dropped**:

```rust
pub struct Unsupported { pub flag: String, pub effect: String }
```

A silently ignored setting is how users conclude the application does nothing.

Detected on the reference machine: version 3.16.28, 97 flags, `-F` present,
`--adaptive-sync` present, `--hdr-enabled` present, `--fsr` **absent**.

### 3.1 Frame limiting, named honestly

Gamescope has two mechanisms and the old code conflated them. It stored a field
called `framerate_limit` and passed it to `-r`, which is `--nested-refresh` —
the refresh rate of the nested display. That *does* cap frames in nested mode, so
it was not broken, but the field name and the UI label promised something else,
and the actual `--framerate-limit` flag (a *divisor* of the refresh rate) was
never used. The type now says what it is:

```rust
pub enum FrameLimit {
    None,
    NestedRefresh(u32),   // -r
}
```

### 3.2 Fullscreen by default

The old builder never emitted `-f` or `-b`, so nested Gamescope — which is what
runs under a KDE Wayland session — opened a small decorated window. `fullscreen`
now defaults to true.

## 4. The compatibility matrix

Legend: **OK** verified on this machine · **OK\*** expected, hardware
unavailable · **CONFLICT** two components doing the same job · **N/A**.

| Combination | Verdict | Notes |
|---|---|---|
| Gamescope + sched-ext | OK\* | different layers entirely; `NOT TESTED` — no `scx_loader` here |
| Gamescope + V-Cache | OK\* | `NOT TESTED` — no X3D CPU here |
| Gamescope + CPU performance | OK | independent; both active during the Booster run |
| Gamescope + GPU DPM high | OK | independent |
| Gamescope + VRR | OK\* | `--adaptive-sync` present in this build; `NOT TESTED` — no VRR-capable display here |
| Gamescope + HDR | OK\* | `--hdr-enabled` present; `NOT TESTED` — both outputs report `HDR: incapable` |
| Gamescope + MangoHud | **CONFLICT** | use `--mangoapp`, never `MANGOHUD=1` as well |
| Gamescope FPS cap + MangoHud `fps_limit` | **CONFLICT** | pick one |
| Gamescope FPS cap + DXVK `dxvk.maxFrameRate` | **CONFLICT** | pick one |
| Gamescope FPS cap + VKD3D limiter | **CONFLICT** | pick one |
| Gamescope FSR + Wine FSR | **CONFLICT** | two upscalers in series |
| Gamescope FSR + `-F nis` | N/A | mutually exclusive by construction — one `-F` value |
| Gamescope + lsfg-vk | **CONFLICT** if both generate frames | lsfg-vk layer is installed here |
| Gamescope + vkBasalt | OK | post-processing inside, presentation outside |
| Gamescope + AMD | OK | verified, RX 9060 XT / RADV |
| Gamescope + NVIDIA | OK\* | `NOT TESTED — hardware unavailable` |
| Gamescope + Intel | OK\* | `NOT TESTED — hardware unavailable` |
| Gamescope + Wayland | OK | nested, verified |
| Gamescope + X11 | OK\* | `NOT TESTED` — session is Wayland |

### 4.1 The frame-limiter rule

Four components can cap frame rate: Gamescope, MangoHud, DXVK and VKD3D. **Only
one may be active.** Two limiters do not average — they beat against each other
and produce exactly the stutter the limiter was meant to remove.

Priority when more than one is available:

1. **Gamescope**, when it is in the pipeline — it is the compositor and has the
   most accurate view of presentation timing.
2. **MangoHud**, when Gamescope is not used but MangoHud is.
3. **DXVK/VKD3D**, last: per-API and therefore invisible to native titles.

### 4.2 Frame generation

Already enforced in `launcher::apply_harmony_policy`: choosing OptiScaler or
AFMF disables lsfg-vk for that game, and choosing lsfg-vk neutralizes OptiScaler
staging and AFMF variables. Two frame generators in series produce doubled and
corrupted frames, not more frames.

## 5. Per-game, not universal

Gamescope should not be a global toggle. Some titles are worse inside it —
overlay problems, input problems, HDR problems — and some simply do not need
scaling. The right shape is:

```
Auto       decide from hardware, session, display and the game profile
Enabled    always wrap
Disabled   never wrap
```

`Auto` has the inputs it needs: `Hardware` knows the session, the render GPU and
the connected displays; `Capabilities` knows which Gamescope flags exist;
`GameProfile.gamescope` carries the per-game override.

**Status: not implemented.** The tri-state and its decision rules are designed,
and the data to drive them is in place, but the current build still exposes
Gamescope as on/off per profile. This is listed in
[FINAL-REPORT.md](FINAL-REPORT.md) under Known Limitations rather than claimed
as done.

## 6. Where the remaining risk is

The Steam bypass. `launcher::is_steam_applaunch_command` returns early for
`steam -applaunch`, skipping the whole pipeline. Since Steam is how most people
start games, the video pipeline is inert in the common case, and
`video_config::write_env_file` only partly compensates — it covers environment
variables, never Gamescope, and only after a re-login.

Doing this properly means writing per-game launch options into Steam's own
config, which is a larger change than this pass. Recorded as audit finding
LNCH-02 and carried into Known Limitations.
