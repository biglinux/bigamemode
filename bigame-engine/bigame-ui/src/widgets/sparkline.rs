//! Sparkline mini-chart: last N data points as a filled line graph.
//!
//! Colors adapt to the active theme: the design's `bgm_chart_color` where it
//! defines one (Gamer), otherwise Adwaita's `accent_color`.
//! Temperature sparklines can override to warm/hot colors via `set_color()`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use gtk4::prelude::*;

/// Maximum data points kept in history.
const MAX_POINTS: usize = 60;

type SparkData = Arc<Mutex<VecDeque<f64>>>;

/// RGB color override: `None` = use Adwaita `accent_color`.
type ColorOverride = Arc<Mutex<Option<(f64, f64, f64)>>>;

/// Sparkline handle: owns the drawing area + data buffer.
///
/// Attach `handle.area` to a widget, push values via `handle.push(value)`.
/// Change line color via `handle.set_color(Some((r, g, b)))`.
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

/// Create a `SparkHandle` with a 120×32px drawing area ready to embed.
#[must_use]
#[allow(clippy::many_single_char_names, clippy::cast_precision_loss)]
pub fn build() -> SparkHandle {
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

        let max = vals.iter().copied().fold(1.0_f64, f64::max);
        let w = f64::from(width);
        let h = f64::from(height);
        let step = w / (vals.len() - 1) as f64;

        // Fill under the line
        cr.set_source_rgba(r, g, b, 0.2);
        cr.move_to(0.0, h);
        for (i, &v) in vals.iter().enumerate() {
            cr.line_to(i as f64 * step, h - (v / max) * (h - 2.0));
        }
        cr.line_to(w, h);
        cr.close_path();
        let _ = cr.fill();

        // Line on top
        cr.set_source_rgba(r, g, b, 0.9);
        cr.set_line_width(1.5);
        for (i, &v) in vals.iter().enumerate() {
            let x = i as f64 * step;
            let y = h - (v / max) * (h - 2.0);
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
