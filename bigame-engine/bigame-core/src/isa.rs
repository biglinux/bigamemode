//! The processor's instruction-set extensions, and the x86-64 level they add
//! up to.
//!
//! For diagnosis: a program built for a newer x86-64 level than the processor
//! has dies with `SIGILL` on the first instruction it lacks. falcond 2.0.14
//! built for x86-64-v3 did exactly that on a Sandy Bridge (AVX, no AVX2 or
//! BMI2), and only these readings tell that apart from any other crash.
//!
//! Read through `std::arch::is_x86_feature_detected!`: CPUID, plus the
//! kernel's consent to the AVX state (XSAVE/OSXSAVE), which a flag list does
//! not show.

/// The extensions that decide which x86-64 level a processor reaches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct CpuIsa {
    /// The x86-64 baseline itself.
    pub sse2: bool,
    /// x86-64-v2: SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B.
    pub sse3: bool,
    pub ssse3: bool,
    pub sse4_1: bool,
    pub sse4_2: bool,
    pub popcnt: bool,
    pub cmpxchg16b: bool,
    /// x86-64-v3: AVX, AVX2, BMI1, BMI2, FMA, F16C, LZCNT, MOVBE.
    pub avx: bool,
    pub avx2: bool,
    pub bmi1: bool,
    pub bmi2: bool,
    pub fma: bool,
    pub f16c: bool,
    pub lzcnt: bool,
    pub movbe: bool,
    /// x86-64-v4: AVX-512 F, BW, CD, DQ and VL.
    pub avx512: bool,
}

impl CpuIsa {
    /// This processor's extensions. All `false` on a non-x86-64 build.
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::is_x86_feature_detected as has;
            Self {
                sse2: has!("sse2"),
                sse3: has!("sse3"),
                ssse3: has!("ssse3"),
                sse4_1: has!("sse4.1"),
                sse4_2: has!("sse4.2"),
                popcnt: has!("popcnt"),
                cmpxchg16b: has!("cmpxchg16b"),
                avx: has!("avx"),
                avx2: has!("avx2"),
                bmi1: has!("bmi1"),
                bmi2: has!("bmi2"),
                fma: has!("fma"),
                f16c: has!("f16c"),
                lzcnt: has!("lzcnt"),
                movbe: has!("movbe"),
                avx512: has!("avx512f")
                    && has!("avx512bw")
                    && has!("avx512cd")
                    && has!("avx512dq")
                    && has!("avx512vl"),
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        Self::default()
    }

    /// The highest x86-64 microarchitecture level every extension of which
    /// this processor has (1–4), or `None` without even the baseline (not an
    /// x86-64 processor).
    #[must_use]
    pub fn level(&self) -> Option<u8> {
        if !self.sse2 {
            return None;
        }
        let v2 =
            self.sse3 && self.ssse3 && self.sse4_1 && self.sse4_2 && self.popcnt && self.cmpxchg16b;
        let v3 = v2
            && self.avx
            && self.avx2
            && self.bmi1
            && self.bmi2
            && self.fma
            && self.f16c
            && self.lzcnt
            && self.movbe;
        Some(match (v2, v3, self.avx512) {
            (true, true, true) => 4,
            (true, true, false) => 3,
            (true, false, _) => 2,
            (false, _, _) => 1,
        })
    }

    /// Whether this processor lacks something of x86-64-v3: a binary built
    /// for v3 (or for a newer CPU) can stop on it with `SIGILL`.
    #[must_use]
    pub fn below_v3(&self) -> bool {
        self.level().is_some_and(|level| level < 3)
    }

    /// The x86-64-v3 extensions this processor lacks, by name.
    #[must_use]
    pub fn missing_for_v3(&self) -> Vec<&'static str> {
        [
            ("AVX", self.avx),
            ("AVX2", self.avx2),
            ("BMI1", self.bmi1),
            ("BMI2", self.bmi2),
            ("FMA", self.fma),
            ("F16C", self.f16c),
            ("LZCNT", self.lzcnt),
            ("MOVBE", self.movbe),
        ]
        .into_iter()
        .filter_map(|(name, has)| (!has).then_some(name))
        .collect()
    }

    /// The extensions support asks about, in the order of the levels, each
    /// with whether this processor has it.
    #[must_use]
    pub fn summary(&self) -> [(&'static str, bool); 8] {
        [
            ("SSE2", self.sse2),
            ("SSE4.1", self.sse4_1),
            ("SSE4.2", self.sse4_2),
            ("AVX", self.avx),
            ("AVX2", self.avx2),
            ("BMI1", self.bmi1),
            ("BMI2", self.bmi2),
            ("FMA", self.fma),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2() -> CpuIsa {
        CpuIsa {
            sse2: true,
            sse3: true,
            ssse3: true,
            sse4_1: true,
            sse4_2: true,
            popcnt: true,
            cmpxchg16b: true,
            ..CpuIsa::default()
        }
    }

    fn v3() -> CpuIsa {
        CpuIsa {
            avx: true,
            avx2: true,
            bmi1: true,
            bmi2: true,
            fma: true,
            f16c: true,
            lzcnt: true,
            movbe: true,
            ..v2()
        }
    }

    #[test]
    fn a_sandy_bridge_is_v2_and_lacks_avx2_and_bmi2() {
        // Core i3-2120: AVX, but none of Haswell's additions.
        let sandy = CpuIsa { avx: true, ..v2() };
        assert_eq!(sandy.level(), Some(2));
        assert!(sandy.below_v3());
        assert_eq!(
            sandy.missing_for_v3(),
            ["AVX2", "BMI1", "BMI2", "FMA", "F16C", "LZCNT", "MOVBE"]
        );
        // Ivy Bridge adds F16C, still no AVX2 or BMI2.
        let ivy = CpuIsa {
            f16c: true,
            ..sandy
        };
        assert_eq!(ivy.level(), Some(2));
        assert!(ivy.missing_for_v3().contains(&"BMI2"));
    }

    #[test]
    fn levels_need_every_extension_of_theirs() {
        assert_eq!(v2().level(), Some(2));
        assert_eq!(v3().level(), Some(3));
        assert_eq!(
            CpuIsa {
                avx512: true,
                ..v3()
            }
            .level(),
            Some(4)
        );
        // AVX-512 without v3 below it does not make v4.
        assert_eq!(
            CpuIsa {
                avx512: true,
                ..v2()
            }
            .level(),
            Some(2)
        );
        // One extension short of v3 (an AMD part without MOVBE, say).
        assert_eq!(
            CpuIsa {
                movbe: false,
                ..v3()
            }
            .level(),
            Some(2)
        );
        // An old AMD K8 or Core 2: the baseline only.
        let k8 = CpuIsa {
            sse2: true,
            sse3: true,
            ..CpuIsa::default()
        };
        assert_eq!(k8.level(), Some(1));
        assert!(k8.below_v3());
        assert!(!v3().below_v3());
        assert!(v3().missing_for_v3().is_empty());
        // Not x86-64 at all.
        assert_eq!(CpuIsa::default().level(), None);
        assert!(!CpuIsa::default().below_v3());
    }

    #[test]
    fn the_summary_names_what_support_asks_about() {
        let names: Vec<_> = v3().summary().iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            [
                "SSE2", "SSE4.1", "SSE4.2", "AVX", "AVX2", "BMI1", "BMI2", "FMA"
            ]
        );
        assert!(v3().summary().iter().all(|(_, has)| *has));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn this_processor_has_at_least_the_baseline() {
        // Every x86-64 processor has SSE2; this binary runs on one.
        assert!(CpuIsa::detect().level().is_some_and(|l| l >= 1));
    }
}
