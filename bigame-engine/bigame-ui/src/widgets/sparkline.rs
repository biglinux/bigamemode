//! Sparkline mini-chart: last N data points as a filled line graph.
//!
//! Colors adapt to the active theme: the design's `bgm_chart_color` where it
//! defines one (Gamer), otherwise Adwaita's `accent_color`.
//! Temperature sparklines can override to warm/hot colors via `set_color()`.
//!
//! The newest point is always at the right edge and every point keeps its
//! width, so the line scrolls rather than stretching while it fills. A
//! chart ([`chart`]) also keeps its vertical scale: a fixed top for a
//! percentage, or a floor for a rate, so its shape says what the reading
//! did and not what the scale did.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use gtk4::prelude::*;

/// Maximum data points kept in history.
const MAX_POINTS: usize = 60;

type SparkData = Arc<Mutex<VecDeque<f64>>>;

/// RGB color override: `None` = use Adwaita `accent_color`.
type ColorOverride = Arc<Mutex<Option<(f64, f64, f64)>>>;

/// The top of a chart's vertical scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    /// Always this value, e.g. 100 for a percentage.
    Fixed(f64),
    /// The highest point shown, but never below this.
    AtLeast(f64),
}

/// Sparkline handle: owns the drawing area + data buffer.
///
/// Attach `handle.area` to a widget, push values via `handle.push(value)`.
/// Change line color via `handle.set_color(Some((r, g, b)))`.
#[derive(Clone)]
pub struct SparkHandle {
    /// The drawing area — add as a widget suffix via `row.add_suffix(&handle.area)`.
    pub area: gtk4::DrawingArea,
    data: SparkData,
    color: ColorOverride,
}

impl SparkHandle {
    /// Push a new value and trigger a redraw.
    pub fn push(&self, value: f64) {
        if let Ok(mut buf) = self.data.lock() {
            if buf.len() >= MAX_POINTS {
                buf.pop_front();
            }
            buf.push_back(value);
        }
        self.area.queue_draw();
    }

    /// Override the sparkline line color (RGB 0.0–1.0).
    ///
    /// Pass `None` to revert to the Adwaita `accent_color`.
    /// Used by the temperature row to signal warm/hot states.
    pub fn set_color(&self, rgb: Option<(f64, f64, f64)>) {
        if let Ok(mut c) = self.color.lock() {
            *c = rgb;
        }
        self.area.queue_draw();
    }
}

/// A chart of `height` pixels on the scale `scale`, with faint guide lines
/// at a quarter, half and three quarters of it.
#[must_use]
pub fn chart(scale: Scale, height: i32) -> SparkHandle {
    let spark = make(scale);
    spark.area.set_content_height(height);
    spark.area.set_vexpand(false);
    spark.area.add_css_class("telemetry-chart");
    spark
}

#[allow(clippy::many_single_char_names, clippy::cast_precision_loss)]
fn make(scale: Scale) -> SparkHandle {
    let data: SparkData = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_POINTS)));
    let color: ColorOverride = Arc::new(Mutex::new(None));
    let area = gtk4::DrawingArea::new();
    area.set_content_height(32);
    area.set_hexpand(true);
    area.add_css_class("sparkline-area");

    let buf = Arc::clone(&data);
    let col = Arc::clone(&color);
    area.set_draw_func(move |widget, cr, width, height| {
        let vals = buf.lock().unwrap_or_else(PoisonError::into_inner);
        let w = f64::from(width);
        let h = f64::from(height);
        let fg = widget.color();
        cr.set_source_rgba(
            f64::from(fg.red()),
            f64::from(fg.green()),
            f64::from(fg.blue()),
            0.08,
        );
        cr.set_line_width(1.0);
        for q in [0.25, 0.5, 0.75] {
            let y = (h * q).round() + 0.5;
            cr.move_to(0.0, y);
            cr.line_to(w, y);
        }
        let _ = cr.stroke();
        if vals.len() < 2 {
            return;
        }

        // Resolve RGB: color override → the design's chart colour (Gamer
        // defines one) → accent_color from theme → fallback blue
        let (r, g, b) = col.lock().ok().and_then(|c| *c).unwrap_or_else(|| {
            #[allow(deprecated)]
            let style = widget.style_context();
            #[allow(deprecated)]
            style
                .lookup_color("bgm_chart_color")
                .or_else(|| style.lookup_color("accent_color"))
                .map_or((0.2, 0.6, 1.0), |rgba| {
                    (
                        f64::from(rgba.red()),
                        f64::from(rgba.green()),
                        f64::from(rgba.blue()),
                    )
                })
        });

        let max = match scale {
            Scale::Fixed(top) => top,
            Scale::AtLeast(floor) => vals.iter().copied().fold(floor, f64::max),
        };
        // A fixed width per point, the newest at the right edge.
        let step = w / (MAX_POINTS - 1) as f64;
        let start = w - (vals.len() - 1) as f64 * step;
        let y_of = |v: f64| h - (v / max).clamp(0.0, 1.0) * (h - 2.0);

        // Fill under the line
        cr.set_source_rgba(r, g, b, 0.2);
        cr.move_to(start, h);
        for (i, &v) in vals.iter().enumerate() {
            cr.line_to(start + i as f64 * step, y_of(v));
        }
        cr.line_to(w, h);
        cr.close_path();
        let _ = cr.fill();

        cr.set_source_rgba(r, g, b, 0.9);
        cr.set_line_width(1.5);
        for (i, &v) in vals.iter().enumerate() {
            let x = start + i as f64 * step;
            let y = y_of(v);
            if i == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        let _ = cr.stroke();
    });

    SparkHandle { area, data, color }
}
