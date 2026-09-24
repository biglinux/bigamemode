# 27 — Graphics compatibility matrix

Which graphics technologies BiGame-mode lets run together for one game, and
why. The same table is code — `bigame-core/src/graphics/rules.rs` — and the
planner and the launch both read it; this page is its explanation.
Development documentation; the application does not read it.

**The rule:** never two technologies doing the same job in series without a
demonstrated reason. Two upscalers in a row scale an already-scaled image;
two frame generators in a row interpolate interpolated frames.

**Verdicts:** SUPPORTED · SUPPORTED WITH CONDITIONS · CONFLICT · EXPERIMENTAL ·
UNKNOWN · BLOCKED. **Basis:** *tested here* (reference machine: RX 9060 XT,
Mesa 26.2.2, Proton Experimental), *upstream* (the project's documentation or
code), *principle* (follows from what the two do), *none*.

| Combination | Verdict | Basis | Why / condition |
|---|---|---|---|
| OptiScaler + anti-cheat | **BLOCKED** | upstream | a DLL injection; OptiScaler says "do not use in multiplayer games"; bans possible |
| OptiScaler frame generation + anti-cheat | **BLOCKED** | upstream | same |
| ReShade + anti-cheat | **BLOCKED** | upstream | the add-on build is for single-player games only |
| RenoDX + anti-cheat | **BLOCKED** | upstream | needs ReShade's add-on build |
| OptiScaler + the game's XeSS | SUPPORTED WITH CONDITIONS | **tested here** | select XeSS in the game; OptiScaler runs its output in its place (SotTR: `init successful for fsr31`) |
| OptiScaler + the game's FSR | SUPPORTED WITH CONDITIONS | upstream | select FSR in the game |
| OptiScaler + the game's DLSS | SUPPORTED WITH CONDITIONS | upstream | select DLSS in the game; on AMD/Intel needs GPU spoofing (Experimental) |
| OptiScaler + Gamescope upscaling | **CONFLICT** | principle | two upscalers in series — let the game render at the display's size |
| OptiScaler + Wine FSR | **CONFLICT** | principle | Wine FSR upscales a lower fullscreen resolution: a second upscaler |
| Native DLSS / FSR / XeSS + Gamescope upscaling | **CONFLICT** | principle | the game already upscales to its output |
| Native DLSS / FSR / XeSS + Wine FSR | **CONFLICT** | principle | two upscalers |
| Gamescope upscaling + Wine FSR | **CONFLICT** | principle | two upscalers |
| OptiScaler frame generation + lsfg-vk | **CONFLICT** | principle | two frame generators |
| Native frame generation + lsfg-vk | **CONFLICT** | principle | two frame generators |
| Native frame generation + OptiScaler frame generation | **CONFLICT** | upstream | OptiScaler's replaces the game's: turn the game's off |
| OptiScaler (upscaling) + lsfg-vk | SUPPORTED WITH CONDITIONS | principle | different jobs; lsfg-vk asks for no other Vulkan layers and has no VRR |
| OptiScaler upscaling + its frame generation | EXPERIMENTAL | upstream | needs its upscaler on; not established here; raises latency |
| OptiScaler + MangoHud | SUPPORTED | **tested here** | overlay shown normally with OptiScaler loaded |
| OptiScaler frame generation + MangoHud | SUPPORTED WITH CONDITIONS | upstream | MangoHud counts generated frames: presented, not rendered |
| lsfg-vk + MangoHud | SUPPORTED WITH CONDITIONS | upstream | MangoHud misses lsfg frames if loaded first |
| OptiScaler + ReShade | SUPPORTED WITH CONDITIONS | upstream | only one can be `dxgi.dll`: ReShade in OptiScaler's `plugins\` or `LoadReshade=true` |
| OptiScaler + RenoDX | EXPERIMENTAL | none | through ReShade loaded by OptiScaler; RenoDX does not support Linux officially |
| RenoDX + HDR | SUPPORTED WITH CONDITIONS | upstream | HDR swapchain (`DXVK_HDR=1`, HDR display and compositor), no AutoHDR/RTX HDR |
| ReShade + RenoDX | SUPPORTED WITH CONDITIONS | upstream | ReShade 6.8+ with full add-on support |
| anything else | UNKNOWN | none | not established either way — shown as such, not as supported |

## What the application does with it

- **Planner** ([29](29-DLSS-IMPLEMENTATION.md)): a plan that would place
  OptiScaler lists the Gamescope upscaling, Wine FSR and lsfg-vk it must turn
  off *for that game*, and reports every non-supported pair among what would
  be active.
- **Launch**: the Harmony Policy applies the plan's list when the game is
  launched, so the global settings stay as they are and the conflict never
  reaches the game.
- **Anti-cheat**: a game with any anti-cheat marker gets no injection at all,
  in Recommended or Advanced; the plan still points to the game's own
  upscaler, which is a menu setting.

## Not tested here

Native DLSS (no NVIDIA GPU), XeSS on Intel hardware, HDR (display not
HDR-capable in this setup), Gamescope nested sessions with OptiScaler, and
lsfg-vk (not installed on the reference machine): NOT TESTED — hardware or
software unavailable. Their verdicts above are upstream's or the principle's.
