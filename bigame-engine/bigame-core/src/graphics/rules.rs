//! Which graphics technologies may run together, and why.
//!
//! The rule is: never two technologies doing the same job in series without a
//! demonstrated reason. Two upscalers in a row scale an already-scaled image;
//! two frame generators in a row interpolate interpolated frames. Each entry
//! says what the verdict rests on — a test on the reference machine, an
//! upstream statement, or the principle — and a pair nobody has established
//! is `Unknown`, not a guess.

use serde::Serialize;

use super::text::N_;

/// A technology that can be active for a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tech {
    /// The game's own DLSS Super Resolution / DLAA.
    NativeDlss,
    /// The game's own FSR upscaling.
    NativeFsr,
    /// The game's own `XeSS` upscaling.
    NativeXess,
    /// The game's own frame generation (DLSS-G, FSR-FG, XeSS-FG).
    NativeFrameGen,
    /// `OptiScaler` replacing the game's upscaler.
    OptiScalerUpscaler,
    /// `OptiScaler`'s frame generation.
    OptiScalerFrameGen,
    /// Gamescope upscaling (render below the output size, `-F` filter).
    GamescopeUpscaling,
    /// Wine's fullscreen FSR (`WINE_FULLSCREEN_FSR`).
    WineFsr,
    /// lsfg-vk frame generation.
    LsfgVk,
    /// `MangoHud` overlay.
    MangoHud,
    /// `ReShade`.
    ReShade,
    /// A `RenoDX` HDR mod (a `ReShade` add-on).
    RenoDx,
    /// HDR output.
    Hdr,
    /// Anti-cheat present in the game.
    AntiCheat,
}

/// How two technologies get along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Works together.
    Supported,
    /// Works together if set up a particular way (the reason says how).
    SupportedWithConditions,
    /// Do not combine: they do the same job, or break each other.
    Conflict,
    /// Reported to work, not established.
    Experimental,
    /// Nobody has established it either way.
    Unknown,
    /// Never: risks the user's account.
    Blocked,
}

/// What a verdict rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Observed on the reference machine.
    TestedHere,
    /// The project's own documentation or code.
    Upstream,
    /// Follows from what the two do (two upscalers in series, …).
    Principle,
    /// Nothing yet.
    None,
}

/// A verdict for a pair, with its reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rule {
    /// First technology.
    pub a: Tech,
    /// Second technology.
    pub b: Tech,
    /// The verdict.
    pub verdict: Verdict,
    /// Why, in a sentence a user can act on.
    pub why: &'static str,
    /// What it rests on.
    pub basis: Basis,
}

use Basis as B;
use Tech as T;
use Verdict as V;

const fn r(a: Tech, b: Tech, verdict: Verdict, basis: Basis, why: &'static str) -> Rule {
    Rule {
        a,
        b,
        verdict,
        why,
        basis,
    }
}

/// Every pair with an established verdict.
pub const RULES: &[Rule] = &[
    // ── Anti-cheat: nothing that injects into the game ─────────────────────
    r(
        T::OptiScalerUpscaler,
        T::AntiCheat,
        V::Blocked,
        B::Upstream,
        N_(
            "OptiScaler is loaded into the game as a DLL; its own documentation says not to use it in online games, and anti-cheat can ban for it",
        ),
    ),
    r(
        T::OptiScalerFrameGen,
        T::AntiCheat,
        V::Blocked,
        B::Upstream,
        N_("OptiScaler is loaded into the game as a DLL; anti-cheat can ban for it"),
    ),
    r(
        T::ReShade,
        T::AntiCheat,
        V::Blocked,
        B::Upstream,
        N_(
            "ReShade's add-on build is for single-player games only; anti-cheat can ban for injected DLLs",
        ),
    ),
    r(
        T::RenoDx,
        T::AntiCheat,
        V::Blocked,
        B::Upstream,
        N_("RenoDX needs ReShade's add-on build, which is for single-player games only"),
    ),
    // ── Upscalers: one at a time ───────────────────────────────────────────
    r(
        T::OptiScalerUpscaler,
        T::NativeXess,
        V::SupportedWithConditions,
        B::TestedHere,
        N_(
            "OptiScaler takes over the game's XeSS: choose XeSS in the game's menu, and OptiScaler runs its own upscaler in its place",
        ),
    ),
    r(
        T::OptiScalerUpscaler,
        T::NativeFsr,
        V::SupportedWithConditions,
        B::Upstream,
        N_("OptiScaler takes over the game's FSR: choose FSR in the game's menu"),
    ),
    r(
        T::OptiScalerUpscaler,
        T::NativeDlss,
        V::SupportedWithConditions,
        B::Upstream,
        N_(
            "OptiScaler takes over the game's DLSS: choose DLSS in the game's menu; on AMD and Intel this needs GPU spoofing",
        ),
    ),
    r(
        T::OptiScalerUpscaler,
        T::GamescopeUpscaling,
        V::Conflict,
        B::Principle,
        N_(
            "two upscalers in series: Gamescope would scale an image OptiScaler has already scaled; let the game render at the display's resolution",
        ),
    ),
    r(
        T::OptiScalerUpscaler,
        T::WineFsr,
        V::Conflict,
        B::Principle,
        N_(
            "Wine's FSR upscales a lower fullscreen resolution — a second upscaler on top of OptiScaler's",
        ),
    ),
    r(
        T::NativeDlss,
        T::GamescopeUpscaling,
        V::Conflict,
        B::Principle,
        N_("two upscalers in series: the game already upscales to its output resolution"),
    ),
    r(
        T::NativeFsr,
        T::GamescopeUpscaling,
        V::Conflict,
        B::Principle,
        N_("two upscalers in series: the game already upscales to its output resolution"),
    ),
    r(
        T::NativeXess,
        T::GamescopeUpscaling,
        V::Conflict,
        B::Principle,
        N_("two upscalers in series: the game already upscales to its output resolution"),
    ),
    r(
        T::NativeDlss,
        T::WineFsr,
        V::Conflict,
        B::Principle,
        N_("two upscalers in series"),
    ),
    r(
        T::NativeFsr,
        T::WineFsr,
        V::Conflict,
        B::Principle,
        N_("two upscalers in series"),
    ),
    r(
        T::NativeXess,
        T::WineFsr,
        V::Conflict,
        B::Principle,
        N_("two upscalers in series"),
    ),
    r(
        T::GamescopeUpscaling,
        T::WineFsr,
        V::Conflict,
        B::Principle,
        N_("two upscalers in series"),
    ),
    // ── Frame generation: one at a time ────────────────────────────────────
    r(
        T::OptiScalerFrameGen,
        T::LsfgVk,
        V::Conflict,
        B::Principle,
        N_("two frame generators in series interpolate interpolated frames"),
    ),
    r(
        T::NativeFrameGen,
        T::LsfgVk,
        V::Conflict,
        B::Principle,
        N_("two frame generators in series interpolate interpolated frames"),
    ),
    r(
        T::NativeFrameGen,
        T::OptiScalerFrameGen,
        V::Conflict,
        B::Upstream,
        N_(
            "OptiScaler's frame generation replaces the game's; keep the game's off when OptiScaler's is on",
        ),
    ),
    // ── Upscaler + a frame generator: different jobs ───────────────────────
    r(
        T::OptiScalerUpscaler,
        T::LsfgVk,
        V::SupportedWithConditions,
        B::Principle,
        N_(
            "different jobs (upscaling, then frame generation), but lsfg-vk upstream asks for no other Vulkan layers and does not support VRR",
        ),
    ),
    r(
        T::OptiScalerUpscaler,
        T::OptiScalerFrameGen,
        V::Experimental,
        B::Upstream,
        N_(
            "OptiScaler's frame generation needs its upscaler on; reported to work, not established here — and it raises latency",
        ),
    ),
    // ── Overlays and effects ───────────────────────────────────────────────
    r(
        T::OptiScalerUpscaler,
        T::MangoHud,
        V::Supported,
        B::TestedHere,
        N_("the overlay showed normally with OptiScaler loaded"),
    ),
    r(
        T::OptiScalerFrameGen,
        T::MangoHud,
        V::SupportedWithConditions,
        B::Upstream,
        N_(
            "MangoHud counts generated frames: the number shown is presented frames, not rendered ones",
        ),
    ),
    r(
        T::LsfgVk,
        T::MangoHud,
        V::SupportedWithConditions,
        B::Upstream,
        N_(
            "MangoHud misses lsfg-vk's frames if it loads before it; lsfg-vk upstream suggests disabling other layers",
        ),
    ),
    r(
        T::OptiScalerUpscaler,
        T::ReShade,
        V::SupportedWithConditions,
        B::Upstream,
        N_(
            "only one can be dxgi.dll: ReShade goes in OptiScaler's plugins folder, or is loaded by OptiScaler (LoadReshade)",
        ),
    ),
    r(
        T::OptiScalerUpscaler,
        T::RenoDx,
        V::Experimental,
        B::None,
        N_(
            "RenoDX runs as a ReShade add-on loaded through OptiScaler; not established, and RenoDX does not support Linux officially",
        ),
    ),
    r(
        T::RenoDx,
        T::Hdr,
        V::SupportedWithConditions,
        B::Upstream,
        N_(
            "needs an HDR swapchain (DXVK_HDR=1 and an HDR-capable display and compositor) and no other HDR conversion (AutoHDR, RTX HDR)",
        ),
    ),
    r(
        T::ReShade,
        T::RenoDx,
        V::SupportedWithConditions,
        B::Upstream,
        N_("RenoDX needs ReShade 6.8+ with full add-on support"),
    ),
];

/// The verdict for a pair, in either order. `Unknown` when nothing is
/// established.
#[must_use]
pub fn check(a: Tech, b: Tech) -> Rule {
    RULES
        .iter()
        .find(|r| (r.a == a && r.b == b) || (r.a == b && r.b == a))
        .cloned()
        .unwrap_or(Rule {
            a,
            b,
            verdict: Verdict::Unknown,
            why: N_("not established either way"),
            basis: Basis::None,
        })
}

/// Every pair among `active` that is not plainly supported, worst first.
#[must_use]
pub fn problems(active: &[Tech]) -> Vec<Rule> {
    let mut out = Vec::new();
    for (i, &a) in active.iter().enumerate() {
        for &b in &active[i + 1..] {
            let rule = check(a, b);
            if !matches!(rule.verdict, Verdict::Supported | Verdict::Unknown) {
                out.push(rule);
            }
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.verdict));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_upscalers_or_two_frame_generators_never_pass() {
        assert_eq!(
            check(T::OptiScalerUpscaler, T::GamescopeUpscaling).verdict,
            V::Conflict
        );
        assert_eq!(
            check(T::GamescopeUpscaling, T::OptiScalerUpscaler).verdict,
            V::Conflict,
            "order does not matter"
        );
        assert_eq!(check(T::OptiScalerFrameGen, T::LsfgVk).verdict, V::Conflict);
        assert_eq!(check(T::NativeFrameGen, T::LsfgVk).verdict, V::Conflict);
        assert_eq!(check(T::NativeDlss, T::WineFsr).verdict, V::Conflict);
    }

    #[test]
    fn injection_into_a_protected_game_is_blocked() {
        for t in [
            T::OptiScalerUpscaler,
            T::OptiScalerFrameGen,
            T::ReShade,
            T::RenoDx,
        ] {
            assert_eq!(check(t, T::AntiCheat).verdict, V::Blocked, "{t:?}");
        }
    }

    #[test]
    fn an_unestablished_pair_is_unknown_not_supported() {
        let r = check(T::Hdr, T::LsfgVk);
        assert_eq!((r.verdict, r.basis), (V::Unknown, B::None));
    }

    #[test]
    fn every_rule_has_a_reason_and_no_pair_is_listed_twice() {
        let mut seen = std::collections::HashSet::new();
        for r in RULES {
            assert!(!r.why.is_empty());
            let key = if r.a <= r.b { (r.a, r.b) } else { (r.b, r.a) };
            assert!(seen.insert(key), "{:?} + {:?} listed twice", r.a, r.b);
        }
    }

    #[test]
    fn problems_are_reported_worst_first_and_harmless_pairs_left_out() {
        let p = problems(&[
            T::OptiScalerUpscaler,
            T::GamescopeUpscaling,
            T::MangoHud,
            T::AntiCheat,
        ]);
        assert_eq!(p[0].verdict, V::Blocked);
        assert!(p.iter().any(|r| r.verdict == V::Conflict));
        assert!(
            !p.iter().any(|r| r.b == T::MangoHud || r.a == T::MangoHud),
            "supported pairs are not problems"
        );
    }
}
