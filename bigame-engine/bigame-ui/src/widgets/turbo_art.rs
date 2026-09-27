//! The Turbo control's artwork, drawn with cairo.
//!
//! One disc with a power symbol, drawn from one number: `level`, from 0
//! (off) to 1 (on).
//!
//! - 0: a dark glass disc, a thin cold rim, the symbol grey.
//! - 0 → 1: the disc fills from the bottom like a liquid, its surface
//!   swaying, in the spectrum (cyan, blue, purple, magenta); the rim lights
//!   clockwise from the top at the same pace.
//! - 1: the rim swirls with the spectrum and glows; the symbol is white with
//!   a cyan glow over the deep, filled disc.
//!
//! The level follows what Turbo is really doing ([`super::booster_button`]):
//! each stage of switching on moves it, never a timer. `time` only turns the
//! rim and sways the liquid.

use std::f64::consts::{PI, TAU};

use gtk4::cairo;

/// A colour tint over the spectrum, for the states that are not plain on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tint {
    /// The spectrum.
    #[default]
    Spectrum,
    /// On, but something did not take: amber.
    Warning,
    /// Could not switch on: red.
    Error,
}

/// What to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// 0 off … 1 on.
    pub level: f64,
    /// Seconds, for what turns and sways.
    pub time: f64,
    /// The colour treatment.
    pub tint: Tint,
    /// The pointer is over the disc.
    pub hover: bool,
}

type Rgb = (f64, f64, f64);

const CYAN: Rgb = (0.13, 0.86, 0.95);
const BLUE: Rgb = (0.24, 0.45, 1.00);
const PURPLE: Rgb = (0.58, 0.34, 0.98);
const MAGENTA: Rgb = (0.96, 0.25, 0.70);
const COLD: Rgb = (0.40, 0.43, 0.50);
const AMBER: Rgb = (0.98, 0.70, 0.18);
const RED: Rgb = (0.95, 0.28, 0.30);
/// The disc's deep blue, what the filled disc settles into.
const DEEP: Rgb = (0.05, 0.08, 0.30);

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t))
}

/// The spectrum at `x` (any real; it wraps): cyan → blue → purple →
/// magenta → cyan.
fn spectrum(x: f64, tint: Tint) -> Rgb {
    match tint {
        Tint::Warning => {
            return mix(
                AMBER,
                (1.0, 0.85, 0.4),
                (x.rem_euclid(1.0) * TAU).sin() * 0.5 + 0.5,
            );
        }
        Tint::Error => return RED,
        Tint::Spectrum => {}
    }
    let stops = [CYAN, BLUE, PURPLE, MAGENTA, CYAN];
    let x = x.rem_euclid(1.0) * 4.0;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = (x.floor() as usize).min(3);
    mix(stops[i], stops[i + 1], x - x.floor())
}

fn smooth(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Draw the whole disc into a `size`×`size` square.
pub fn draw(cr: &cairo::Context, size: f64, look: Look) {
    let c = size / 2.0;
    let r = size * 0.40;
    cr.save().ok();
    cr.translate(c, c);
    halo(cr, r, look);
    body(cr, r, look);
    liquid(cr, r, look);
    glass(cr, r);
    rim(cr, r, look);
    symbol(cr, r, look);
    cr.restore().ok();
}

/// The glow round the disc while it is on: one smooth ring in the colour
/// passing the top of the rim.
fn halo(cr: &cairo::Context, r: f64, look: Look) {
    let on = smooth(0.2, 1.0, look.level);
    if on <= 0.0 {
        return;
    }
    let col = spectrum(look.time * 0.25, look.tint);
    let outer = r * 1.22;
    let g = cairo::RadialGradient::new(0.0, 0.0, r * 0.9, 0.0, 0.0, outer);
    g.add_color_stop_rgba(0.0, col.0, col.1, col.2, 0.0);
    g.add_color_stop_rgba(0.35, col.0, col.1, col.2, 0.30 * on);
    g.add_color_stop_rgba(1.0, col.0, col.1, col.2, 0.0);
    let _ = cr.set_source(&g);
    cr.arc(0.0, 0.0, outer, 0.0, TAU);
    let _ = cr.fill();
}

/// The disc itself: dark glass, lifted a little under the pointer.
fn body(cr: &cairo::Context, r: f64, look: Look) {
    let g = cairo::RadialGradient::new(-r * 0.3, -r * 0.4, r * 0.1, 0.0, 0.0, r);
    let lift = if look.hover { 0.035 } else { 0.0 };
    let hi = (0.17 + lift, 0.18 + lift, 0.24 + lift);
    let lo = (0.05, 0.06, 0.10);
    g.add_color_stop_rgb(0.0, hi.0, hi.1, hi.2);
    g.add_color_stop_rgb(1.0, lo.0, lo.1, lo.2);
    cr.arc(0.0, 0.0, r, 0.0, TAU);
    let _ = cr.set_source(&g);
    let _ = cr.fill();
}

/// The swaying surface of the liquid, as a path across the disc.
fn surface_path(cr: &cairo::Context, r: f64, wave: impl Fn(f64) -> f64) {
    let steps = 48;
    cr.move_to(-r - 2.0, wave(-r - 2.0));
    for s in 0..=steps {
        let x = -r - 2.0 + (2.0 * r + 4.0) * f64::from(s) / f64::from(steps);
        cr.line_to(x, wave(x));
    }
}

/// The liquid rising in the disc: a swaying surface, the spectrum below
/// it, settling into the deep blue as the disc fills.
fn liquid(cr: &cairo::Context, r: f64, look: Look) {
    if look.level <= 0.0 {
        return;
    }
    let level = look.level.min(1.0);
    // The surface: at 1 it is above the disc, and the sway has died down.
    let surface = r - level * 2.0 * r - r * 0.12 * smooth(0.85, 1.0, level);
    let sway = r * 0.055 * (1.0 - smooth(0.7, 1.0, level));
    let t = look.time;
    let wave = |x: f64| {
        surface
            + sway * ((x / r) * 3.1 + t * 2.3).sin()
            + sway * 0.5 * ((x / r) * 5.7 - t * 1.7).sin()
    };
    cr.save().ok();
    cr.arc(0.0, 0.0, r - 0.5, 0.0, TAU);
    cr.clip();
    // The body of the liquid: the spectrum, drifting, darkened towards the
    // bottom; more of the deep blue the fuller the disc.
    let deep = smooth(0.55, 1.0, level);
    let top = mix(spectrum(t * 0.05, look.tint), DEEP, deep * 0.75);
    let bottom = mix(
        spectrum(t * 0.05 + 0.45, look.tint),
        DEEP,
        0.55 + deep * 0.35,
    );
    let g = cairo::LinearGradient::new(0.0, surface, 0.0, r);
    g.add_color_stop_rgba(0.0, top.0, top.1, top.2, 0.92);
    g.add_color_stop_rgba(1.0, bottom.0, bottom.1, bottom.2, 0.96);
    let _ = cr.set_source(&g);
    surface_path(cr, r, wave);
    cr.line_to(r + 2.0, r + 2.0);
    cr.line_to(-r - 2.0, r + 2.0);
    cr.close_path();
    let _ = cr.fill();
    // The crest: a line of light on the surface, gone once full.
    let crest = 1.0 - smooth(0.8, 1.0, level);
    if crest > 0.0 {
        let col = spectrum(t * 0.05 - 0.1, look.tint);
        for (w, a) in [(r * 0.06, 0.18), (r * 0.02, 0.9)] {
            cr.set_line_width(w);
            cr.set_source_rgba(
                lerp(col.0, 1.0, 0.45),
                lerp(col.1, 1.0, 0.45),
                lerp(col.2, 1.0, 0.45),
                a * crest,
            );
            surface_path(cr, r, wave);
            let _ = cr.stroke();
        }
    }
    // Once full, a soft light at the centre for the symbol to sit in.
    if deep > 0.0 {
        let col = spectrum(t * 0.05 + 0.2, look.tint);
        let g = cairo::RadialGradient::new(0.0, 0.0, 0.0, 0.0, 0.0, r * 0.75);
        g.add_color_stop_rgba(0.0, col.0, col.1, col.2, 0.32 * deep);
        g.add_color_stop_rgba(1.0, col.0, col.1, col.2, 0.0);
        let _ = cr.set_source(&g);
        cr.arc(0.0, 0.0, r, 0.0, TAU);
        let _ = cr.fill();
    }
    cr.restore().ok();
}

/// A glass highlight across the top of the disc.
fn glass(cr: &cairo::Context, r: f64) {
    cr.save().ok();
    cr.arc(0.0, 0.0, r - 0.5, 0.0, TAU);
    cr.clip();
    let g = cairo::LinearGradient::new(0.0, -r, 0.0, -r * 0.1);
    g.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.14);
    g.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    let _ = cr.set_source(&g);
    cr.save().ok();
    cr.translate(0.0, -r * 0.55);
    cr.scale(1.0, 0.5);
    cr.arc(0.0, 0.0, r * 0.85, 0.0, TAU);
    cr.restore().ok();
    let _ = cr.fill();
    cr.restore().ok();
}

/// The rim: thin and cold off; a swirling spectrum on, lit clockwise from
/// the top as the level rises.
fn rim(cr: &cairo::Context, r: f64, look: Look) {
    let on = smooth(0.0, 1.0, look.level);
    let width = lerp(r * 0.035, r * 0.06, on);
    cr.set_line_width(width);
    let cold = if look.tint == Tint::Error { RED } else { COLD };
    cr.set_source_rgba(cold.0, cold.1, cold.2, lerp(0.6, 0.12, on));
    cr.arc(0.0, 0.0, r, 0.0, TAU);
    let _ = cr.stroke();
    if on <= 0.0 {
        return;
    }
    let turn = look.time * 0.25;
    let segments = 120;
    cr.set_line_cap(cairo::LineCap::Butt);
    for s in 0..segments {
        let a0 = f64::from(s) / f64::from(segments);
        if a0 > on {
            break;
        }
        let col = spectrum(a0 + turn, look.tint);
        let start = -PI / 2.0 + a0 * TAU;
        cr.arc(
            0.0,
            0.0,
            r,
            start,
            start + TAU / f64::from(segments) + 0.008,
        );
        cr.set_source_rgba(col.0, col.1, col.2, 0.95);
        let _ = cr.stroke();
    }
}

/// The power symbol: an open ring and a bar, grey off, white with a cyan
/// glow on.
fn symbol(cr: &cairo::Context, r: f64, look: Look) {
    let on = smooth(0.0, 1.0, look.level);
    let ri = r * 0.30;
    let w = r * 0.075;
    // Half the opening at the top, in radians.
    let gap = 0.62;
    let path = |cr: &cairo::Context| {
        cr.new_path();
        cr.arc(0.0, 0.0, ri, -PI / 2.0 + gap, -PI / 2.0 - gap + TAU);
        cr.new_sub_path();
        cr.move_to(0.0, -ri * 1.2);
        cr.line_to(0.0, -ri * 0.2);
    };
    cr.set_line_cap(cairo::LineCap::Round);
    let glow = match look.tint {
        Tint::Spectrum => CYAN,
        Tint::Warning => AMBER,
        Tint::Error => RED,
    };
    // The glow, on: wide and faint, then tighter.
    if on > 0.0 {
        for (mul, alpha) in [(4.0, 0.10), (2.4, 0.22), (1.5, 0.35)] {
            cr.set_line_width(w * mul);
            cr.set_source_rgba(glow.0, glow.1, glow.2, alpha * on);
            path(cr);
            let _ = cr.stroke();
        }
    }
    let off_col = if look.tint == Tint::Error {
        RED
    } else {
        (0.62, 0.65, 0.72)
    };
    let core = mix(off_col, (0.97, 0.99, 1.0), on);
    cr.set_line_width(w);
    cr.set_source_rgba(core.0, core.1, core.2, lerp(0.9, 1.0, on));
    path(cr);
    let _ = cr.stroke();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spectrum_wraps_and_stays_in_range() {
        for i in -20..20 {
            let (r, g, b) = spectrum(f64::from(i) * 0.137, Tint::Spectrum);
            for v in [r, g, b] {
                assert!((0.0..=1.0).contains(&v));
            }
        }
        assert_eq!(spectrum(0.0, Tint::Spectrum), spectrum(1.0, Tint::Spectrum));
    }

    /// Renders the disc's stages for a look at them:
    /// `BIGAME_ART_DIR=/some/dir cargo test -p bigame-ui turbo_art -- --ignored`
    /// writes Netpbm files there.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn render_the_stages() {
        let Some(dir) = std::env::var_os("BIGAME_ART_DIR") else {
            return;
        };
        for (name, level, tint) in [
            ("1-off", 0.0, Tint::Spectrum),
            ("2-filling", 0.3, Tint::Spectrum),
            ("3-nearly", 0.75, Tint::Spectrum),
            ("4-on", 1.0, Tint::Spectrum),
            ("partial", 1.0, Tint::Warning),
            ("error", 0.0, Tint::Error),
        ] {
            let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 300, 300).unwrap();
            let cr = cairo::Context::new(&surface).unwrap();
            cr.set_source_rgb(0.08, 0.09, 0.16);
            cr.paint().unwrap();
            draw(
                &cr,
                300.0,
                Look {
                    level,
                    time: 2.3,
                    tint,
                    hover: false,
                },
            );
            drop(cr);
            surface.flush();
            let stride = usize::try_from(surface.stride()).unwrap();
            let data = surface.take_data().unwrap();
            let mut ppm = b"P6 300 300 255\n".to_vec();
            for row in data.chunks(stride).take(300) {
                for px in row.chunks(4).take(300) {
                    ppm.extend_from_slice(&[px[2], px[1], px[0]]);
                }
            }
            std::fs::write(std::path::Path::new(&dir).join(format!("{name}.ppm")), ppm).unwrap();
        }
    }

    #[test]
    fn every_level_draws_without_error() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 200, 200).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        for tint in [Tint::Spectrum, Tint::Warning, Tint::Error] {
            for level in [0.0, 0.1, 0.3, 0.6, 0.9, 1.0] {
                draw(
                    &cr,
                    200.0,
                    Look {
                        level,
                        time: 3.7,
                        tint,
                        hover: true,
                    },
                );
                assert_eq!(cr.status(), Ok(()));
            }
        }
    }
}
