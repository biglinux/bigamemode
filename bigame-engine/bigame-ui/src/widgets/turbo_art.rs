//! The Turbo control's artwork, drawn with cairo.
//!
//! One disc, drawn from one number: `level`, from 0 (off) to 1 (on).
//!
//! - 0: a cold grey disc under a 3D wireframe sphere.
//! - up to 0.35: sparks of blue light blink on the mesh's vertices.
//! - up to 1: the mesh fills with the spectrum (cyan, blue, purple, magenta)
//!   and starts to turn.
//! - 1: a spectrum rim swirling round the disc, and a neon whirl at its
//!   centre.
//!
//! The level follows what Turbo is really doing ([`super::booster_button`]):
//! each stage of switching on moves it, never a timer. `time` only turns
//! what is already there.

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
    /// Seconds, for what turns.
    pub time: f64,
    /// The colour treatment.
    pub tint: Tint,
    /// The pointer is over the disc.
    pub hover: bool,
}

type Rgb = (f64, f64, f64);

const CYAN: Rgb = (0.13, 0.86, 0.95);
const BLUE: Rgb = (0.20, 0.45, 1.00);
const PURPLE: Rgb = (0.58, 0.34, 0.98);
const MAGENTA: Rgb = (0.96, 0.25, 0.70);
const COLD: Rgb = (0.42, 0.45, 0.50);
const AMBER: Rgb = (0.98, 0.70, 0.18);
const RED: Rgb = (0.95, 0.28, 0.30);

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

/// A stable pseudo-random number in 0..1 for a mesh vertex.
fn hash(i: i32, j: i32) -> f64 {
    let n = i.wrapping_mul(374_761_393) ^ j.wrapping_mul(668_265_263);
    let n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    f64::from((n ^ (n >> 16)) & 0xffff) / 65535.0
}

/// Draw the whole disc into a `size`×`size` square.
pub fn draw(cr: &cairo::Context, size: f64, look: Look) {
    let c = size / 2.0;
    let r = size * 0.43;
    cr.save().ok();
    cr.translate(c, c);
    body(cr, r, look);
    mesh(cr, r * 0.86, look);
    sparks(cr, r * 0.86, look);
    whirl(cr, r * 0.55, look);
    rim(cr, r, look);
    cr.restore().ok();
}

/// The disc itself: cold metal, warming to a deep violet as it comes on.
fn body(cr: &cairo::Context, r: f64, look: Look) {
    let on = smooth(0.35, 1.0, look.level);
    let g = cairo::RadialGradient::new(-r * 0.3, -r * 0.35, r * 0.1, 0.0, 0.0, r);
    let hi = mix((0.24, 0.26, 0.30), (0.20, 0.14, 0.34), on);
    let lo = mix((0.07, 0.08, 0.10), (0.05, 0.03, 0.12), on);
    let lift = if look.hover { 0.04 } else { 0.0 };
    g.add_color_stop_rgb(0.0, hi.0 + lift, hi.1 + lift, hi.2 + lift);
    g.add_color_stop_rgb(1.0, lo.0, lo.1, lo.2);
    cr.arc(0.0, 0.0, r, 0.0, TAU);
    let _ = cr.set_source(&g);
    let _ = cr.fill();
}

/// Project a point of the unit sphere, turned by `spin` about the vertical
/// axis and tipped towards the viewer: (x, y, depth), depth > 0 in front.
fn project(lat: f64, lon: f64, spin: f64) -> (f64, f64, f64) {
    let tilt: f64 = 0.42;
    let (sl, cl) = lat.sin_cos();
    let (so, co) = (lon + spin).sin_cos();
    let x = cl * so;
    let y0 = sl;
    let z0 = cl * co;
    let (st, ct) = tilt.sin_cos();
    (x, y0 * ct - z0 * st, y0 * st + z0 * ct)
}

/// The wireframe sphere: parallels and meridians, the far side dimmer.
fn mesh(cr: &cairo::Context, r: f64, look: Look) {
    let fill = smooth(0.35, 1.0, look.level);
    let spin = look.time * 0.35 * fill;
    cr.set_line_width(1.0);
    cr.set_line_cap(cairo::LineCap::Round);
    // Each line in two passes, back then front, one stroke per pass.
    for front in [false, true] {
        let alpha = if front { 0.55 } else { 0.18 };
        for i in -5..=5 {
            let lat = f64::from(i) * PI / 12.0;
            let hue = spectrum(f64::from(i + 5) / 11.0 + look.time * 0.05, look.tint);
            let col = mix(COLD, hue, fill);
            path_along(cr, r, 64, front, |t| project(lat, t * TAU, spin));
            cr.set_source_rgba(col.0, col.1, col.2, alpha * (0.6 + 0.4 * fill));
            let _ = cr.stroke();
        }
        for k in 0..12 {
            let lon = f64::from(k) * TAU / 12.0;
            let hue = spectrum(f64::from(k) / 12.0 + look.time * 0.05, look.tint);
            let col = mix(COLD, hue, fill);
            path_along(cr, r, 32, front, |t| project(-PI / 2.0 + t * PI, lon, spin));
            cr.set_source_rgba(col.0, col.1, col.2, alpha * (0.6 + 0.4 * fill));
            let _ = cr.stroke();
        }
    }
}

/// One mesh line as a path, keeping only the segments on the side asked for.
fn path_along(
    cr: &cairo::Context,
    r: f64,
    steps: u32,
    front: bool,
    at: impl Fn(f64) -> (f64, f64, f64),
) {
    let mut open = false;
    for s in 0..=steps {
        let (x, y, z) = at(f64::from(s) / f64::from(steps));
        if (z >= 0.0) == front {
            if open {
                cr.line_to(x * r, y * r);
            } else {
                cr.move_to(x * r, y * r);
                open = true;
            }
        } else {
            open = false;
        }
    }
}

/// Blue sparks blinking on the mesh while Turbo starts.
fn sparks(cr: &cairo::Context, radius: f64, look: Look) {
    // Most at the start of the transition, fading as the colours take over.
    let amount = smooth(0.02, 0.2, look.level) * (1.0 - smooth(0.6, 1.0, look.level));
    if amount <= 0.0 {
        return;
    }
    let spin = look.time * 0.35 * smooth(0.35, 1.0, look.level);
    for i in -5..=5 {
        for j in 0..12 {
            let seed = hash(i, j);
            // Each vertex blinks on its own beat.
            let phase = (look.time * (0.6 + seed) + seed * 7.0).rem_euclid(1.0);
            let lit = (1.0 - (phase * 2.0 - 1.0).abs()).powi(3) * amount;
            if lit < 0.05 || seed > 0.25 + amount * 0.6 {
                continue;
            }
            let (px, py, depth) =
                project(f64::from(i) * PI / 12.0, f64::from(j) * TAU / 12.0, spin);
            if depth < 0.0 {
                continue;
            }
            let (px, py) = (px * radius, py * radius);
            let glow = cairo::RadialGradient::new(px, py, 0.0, px, py, 7.0);
            glow.add_color_stop_rgba(0.0, 0.75, 0.95, 1.0, lit);
            glow.add_color_stop_rgba(0.35, CYAN.0, CYAN.1, CYAN.2, lit * 0.8);
            glow.add_color_stop_rgba(1.0, BLUE.0, BLUE.1, BLUE.2, 0.0);
            let _ = cr.set_source(&glow);
            cr.arc(px, py, 7.0, 0.0, TAU);
            let _ = cr.fill();
        }
    }
}

/// The neon whirl at the centre: spiral arms of light turning slowly.
fn whirl(cr: &cairo::Context, r: f64, look: Look) {
    let on = smooth(0.55, 1.0, look.level);
    if on <= 0.0 {
        return;
    }
    let turn = look.time * 0.6;
    // A soft glow underneath.
    let glow = cairo::RadialGradient::new(0.0, 0.0, 0.0, 0.0, 0.0, r);
    let core = spectrum(look.time * 0.04, look.tint);
    glow.add_color_stop_rgba(0.0, core.0, core.1, core.2, 0.35 * on);
    glow.add_color_stop_rgba(1.0, core.0, core.1, core.2, 0.0);
    let _ = cr.set_source(&glow);
    cr.arc(0.0, 0.0, r, 0.0, TAU);
    let _ = cr.fill();
    cr.set_line_cap(cairo::LineCap::Round);
    for arm in 0..5 {
        let base = f64::from(arm) * TAU / 5.0 + turn;
        let col = spectrum(f64::from(arm) / 5.0 + look.time * 0.05, look.tint);
        // Log spiral from the centre outwards, thinning as it goes.
        for pass in [(6.0, 0.10), (2.2, 0.75)] {
            cr.set_line_width(pass.0);
            cr.move_to(0.0, 0.0);
            for s in 1..=40 {
                let t = f64::from(s) / 40.0;
                let a = base + t * 3.4;
                let d = r * t.powf(1.25);
                cr.line_to(a.cos() * d, a.sin() * d);
            }
            cr.set_source_rgba(col.0, col.1, col.2, pass.1 * on);
            let _ = cr.stroke();
        }
    }
}

/// The rim: cold grey off; a swirling spectrum with a glow on.
fn rim(cr: &cairo::Context, r: f64, look: Look) {
    let on = smooth(0.35, 1.0, look.level);
    let width = lerp(3.0, 7.0, on);
    // Off, or the part of the rim not yet lit.
    cr.set_line_width(width);
    let cold = if look.tint == Tint::Error { RED } else { COLD };
    cr.set_source_rgba(cold.0, cold.1, cold.2, lerp(0.55, 0.15, on));
    cr.arc(0.0, 0.0, r, 0.0, TAU);
    let _ = cr.stroke();
    if on <= 0.0 {
        return;
    }
    let turn = look.time * 0.25;
    let segments = 96;
    // The glow: one smooth ring (segments would show their seams), in the
    // colour passing the top of the rim.
    let glow = spectrum(turn, look.tint);
    let halo = cairo::RadialGradient::new(0.0, 0.0, r - width * 2.0, 0.0, 0.0, r + width * 3.5);
    halo.add_color_stop_rgba(0.0, glow.0, glow.1, glow.2, 0.0);
    halo.add_color_stop_rgba(0.45, glow.0, glow.1, glow.2, 0.28 * on);
    halo.add_color_stop_rgba(1.0, glow.0, glow.1, glow.2, 0.0);
    let _ = cr.set_source(&halo);
    cr.arc(0.0, 0.0, r + width * 3.5, 0.0, TAU);
    cr.arc_negative(0.0, 0.0, (r - width * 2.0).max(0.0), TAU, 0.0);
    let _ = cr.fill();
    // Then the rim itself, lit from the top, clockwise, as the level rises.
    for (w, alpha) in [(width, 0.95)] {
        cr.set_line_width(w);
        cr.set_line_cap(cairo::LineCap::Butt);
        for s in 0..segments {
            let a0 = f64::from(s) / f64::from(segments);
            if a0 > on {
                break;
            }
            let col = spectrum(a0 + turn, look.tint);
            let start = -PI / 2.0 + a0 * TAU;
            cr.arc(0.0, 0.0, r, start, start + TAU / f64::from(segments) + 0.01);
            cr.set_source_rgba(col.0, col.1, col.2, alpha * on);
            let _ = cr.stroke();
        }
    }
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

    /// Renders the disc's stages to PNG files for a look at them:
    /// `BIGAME_ART_DIR=/some/dir cargo test -p bigame-ui turbo_art -- --ignored`.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn render_the_stages() {
        let Some(dir) = std::env::var_os("BIGAME_ART_DIR") else {
            return;
        };
        for (name, level, tint) in [
            ("1-off", 0.0, Tint::Spectrum),
            ("2-sparks", 0.18, Tint::Spectrum),
            ("3-filling", 0.6, Tint::Spectrum),
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
            // Netpbm: no image library needed; BGRA rows to RGB.
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
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 300, 300).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        for tint in [Tint::Spectrum, Tint::Warning, Tint::Error] {
            for level in [0.0, 0.1, 0.3, 0.6, 1.0] {
                draw(
                    &cr,
                    300.0,
                    Look {
                        level,
                        time: 3.7,
                        tint,
                        hover: false,
                    },
                );
                assert_eq!(cr.status(), Ok(()));
            }
        }
    }
}
