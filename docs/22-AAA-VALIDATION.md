# 22 — AAA validation

What was checked against the real games installed on the reference machine
in this pass, beyond the benchmark sessions of [13](13-AAA-BENCHMARKS.md).

| Title | Check | Status |
|---|---|---|
| Shadow of the Tomb Raider | Identified while running: `SOTTR.exe`, Steam app 750920, Proton - Experimental, VKD3D-Proton · DX12, `card1`, the live prefix on the games disk (not the stale one in the home library) | VERIFIED |
| Shadow of the Tomb Raider | Detection cost | MEASURED — 9–10 ms, no forks |
| Shadow of the Tomb Raider | Home card: cover from the local Steam cache, time running, runtime/graphics/process, "no profile of its own yet · using falcond's general Proton profile" | VERIFIED on screen |
| Shadow of the Tomb Raider | falcond has no specific profile for it — only the generic Proton fallback applies | VERIFIED (`matching_profile` → none; status `ACTIVE_PROFILE: Proton`) |
| Shadow of the Tomb Raider | Profile created from the offer, then verified active | VERIFIED — created by clicking the offer (02:55); at the next launch falcond matched `SOTTR.exe` and reported it active |
| Shadow of the Tomb Raider | Game graphics settings restored after the CPU-bound sessions (a DirectX 12 slip during restoration caught and undone) | VERIFIED — settings block identical to the clean session's |
| Cyberpunk 2077 | Result files parsed and cross-checked | VERIFIED (earlier pass) |
| Cyberpunk 2077 | Identified while running | NOT TESTED |
| Rise of the Tomb Raider | Where the Windows build writes its results | NOT TESTED — not seen yet; the provider does not guess |
| Tomb Raider (2013) | — | NOT TESTED |
| Unigine Superposition | — | NOT TESTED in this pass |

## falcond, observed during real games

From its journal on 2026-09-23/24: every Proton game got the **handheld**
`Proton` profile (`scx=lavd mode=power perf=false vcache=cache
inhibit=true`); every scheduler switch failed (`ServiceUnknown` — no
`scx_loader`); activation and deactivation followed each real launch and
exit. The only effective action was idle inhibit. [14](14-TURBO-AUDIT.md).
