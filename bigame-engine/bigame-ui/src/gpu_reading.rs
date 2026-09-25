//! A GPU reading as the pages show it: what was measured, and plainly
//! nothing where the driver reports nothing.

use bigame_core::gpu_telemetry::{ClockLimit, GpuSample};

use crate::i18n::i18n;

/// Clock and load, e.g. `1721 MHz · 99%`.
pub fn load_text(s: &GpuSample) -> String {
    if s.asleep {
        return i18n("Asleep");
    }
    match (s.clock_mhz, s.busy_pct) {
        (Some(c), Some(b)) => format!("{c} MHz · {b}%"),
        (Some(c), None) => format!("{c} MHz"),
        (None, Some(b)) => format!("{b}%"),
        (None, None) => i18n("N/A"),
    }
}

/// Temperature with what holds the clock down, and the colour class for it.
pub fn temp_text(s: &GpuSample) -> (String, &'static str) {
    if s.asleep {
        return (i18n("Asleep"), "temp-normal");
    }
    let Some(t) = s.temp_c else {
        return (i18n("Not reported by the driver"), "temp-normal");
    };
    #[allow(clippy::cast_possible_truncation)]
    let c = t.round() as i64;
    let class = match c {
        ..=60 => "temp-normal",
        61..=80 => "temp-warm",
        _ => "temp-hot",
    };
    let mut text = format!("{c}°C");
    for l in &s.limited_by {
        text.push_str(" · ");
        text.push_str(&match l {
            ClockLimit::PowerCap => i18n("power limit"),
            ClockLimit::Thermal => i18n("temperature limit"),
            ClockLimit::HardwareSlowdown => i18n("hardware slowdown"),
        });
    }
    (text, class)
}

/// The number a sparkline follows: load when the driver reports it, else the
/// clock.
pub fn spark_value(s: &GpuSample) -> Option<f64> {
    s.busy_pct
        .map(f64::from)
        .or_else(|| s.clock_mhz.map(f64::from))
}
