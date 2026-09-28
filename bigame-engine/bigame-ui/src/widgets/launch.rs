//! The launch-setting rows Tuning and a game's profile share.
//!
//! Gamescope's filter, Wine FSR's quality, vkBasalt's and `MangoHud`'s looks:
//! the same choices, the same words and the same ⓘ on both pages. Tuning
//! shows the general value; a game's profile puts "General configuration"
//! first ([`INHERIT`]), which leaves the value to Tuning.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::models::{GamescopeFilter, WineFsrMode};

use crate::i18n::i18n;
use crate::widgets::info::{self, Entry};
use crate::widgets::optimization::Picker;

/// The value of the item that leaves a setting to the general
/// configuration.
pub const INHERIT: &str = "general";

/// The first item of a game's row: the general configuration.
fn inherit_item() -> (String, String) {
    (INHERIT.to_owned(), i18n("General configuration"))
}

/// A row's icon, before its title.
pub fn icon(row: &impl IsA<adw::PreferencesRow>, name: &str) {
    let image = gtk4::Image::from_icon_name(name);
    if let Some(r) = row.dynamic_cast_ref::<adw::ActionRow>() {
        r.add_prefix(&image);
    }
}

// ── Gamescope's filter ──────────────────────────────────────────────────────

/// A filter's name.
#[must_use]
pub fn filter_name(f: GamescopeFilter) -> String {
    match f {
        GamescopeFilter::Fsr => "FSR 1.0 (FidelityFX)".to_owned(),
        GamescopeFilter::Nis => "NIS (NVIDIA Image Scaling)".to_owned(),
        GamescopeFilter::Integer => i18n("Integer scaling"),
    }
}

fn filter_id(f: GamescopeFilter) -> &'static str {
    match f {
        GamescopeFilter::Fsr => "fsr",
        GamescopeFilter::Nis => "nis",
        GamescopeFilter::Integer => "integer",
    }
}

/// The filter an item stands for; `None` for the general configuration.
#[must_use]
pub fn filter_of(id: &str) -> Option<GamescopeFilter> {
    match id {
        "fsr" => Some(GamescopeFilter::Fsr),
        "nis" => Some(GamescopeFilter::Nis),
        "integer" => Some(GamescopeFilter::Integer),
        _ => None,
    }
}

/// What each Gamescope filter does, for its ⓘ.
fn filter_entries() -> Vec<Entry> {
    vec![
        Entry {
            title: "FSR 1.0 (FidelityFX Super Resolution)".to_owned(),
            body: i18n(
                "AMD's upscaler: it enlarges the image and sharpens the edges so it does not look blurred. It works on any GPU. The usual choice: render the game smaller and let FSR bring it up to the screen. The sharpness below applies to it.",
            ),
        },
        Entry {
            title: "NIS (NVIDIA Image Scaling)".to_owned(),
            body: i18n(
                "NVIDIA's upscaler, with its own sharpening. It also works on AMD and Intel GPUs. The look is a little different from FSR's: try it when FSR seems too sharp or too soft in a game.",
            ),
        },
        Entry {
            title: i18n("Integer scaling"),
            body: i18n(
                "Each pixel becomes an exact block of 2 × 2, 3 × 3… pixels: no blur and no filter. Made for pixel art and old games. It looks right when the output size is an exact multiple of the render size, such as 1280 × 720 on a 2560 × 1440 screen.",
            ),
        },
    ]
}

/// The upscaling filter row, with its ⓘ. `inherit` puts the general
/// configuration first (a game's row); `value` `None` selects it.
#[must_use]
pub fn filter_picker(inherit: bool, value: Option<GamescopeFilter>) -> Picker {
    let mut items: Vec<(String, String)> = inherit.then(inherit_item).into_iter().collect();
    items.extend(
        [
            GamescopeFilter::Fsr,
            GamescopeFilter::Nis,
            GamescopeFilter::Integer,
        ]
        .map(|f| (filter_id(f).to_owned(), filter_name(f))),
    );
    let picker = Picker::new(
        &i18n("Upscaling filter"),
        &i18n("Used when the render size is below the output size"),
        &items,
        value.map_or(INHERIT, filter_id),
    );
    picker.row.add_suffix(&info::dialog_button(
        &i18n("About the upscaling filters"),
        || {
            (
                i18n("Upscaling filters"),
                i18n(
                    "Gamescope enlarges the image only when the render size is smaller than the output size; then the filter decides how the enlarged image looks.",
                ),
                filter_entries(),
            )
        },
    ));
    picker
}

// ── Wine FSR's quality ──────────────────────────────────────────────────────

/// A Wine FSR mode's name.
#[must_use]
pub fn wine_fsr_mode_name(m: WineFsrMode) -> String {
    match m {
        WineFsrMode::Performance => i18n("Performance"),
        WineFsrMode::Balanced => i18n("Balanced"),
        WineFsrMode::Quality => i18n("Quality"),
        WineFsrMode::Ultra => i18n("Ultra"),
    }
}

/// The mode an item stands for; `None` for the general configuration.
#[must_use]
pub fn wine_fsr_mode_of(id: &str) -> Option<WineFsrMode> {
    match id {
        "performance" => Some(WineFsrMode::Performance),
        "balanced" => Some(WineFsrMode::Balanced),
        "quality" => Some(WineFsrMode::Quality),
        "ultra" => Some(WineFsrMode::Ultra),
        _ => None,
    }
}

/// The Wine FSR quality row, with the ⓘ that gives each mode's size on
/// this screen. `inherit` puts the general configuration first.
#[must_use]
pub fn wine_fsr_mode_picker(inherit: bool, value: Option<WineFsrMode>) -> Picker {
    let mut items: Vec<(String, String)> = inherit.then(inherit_item).into_iter().collect();
    items.extend(
        [
            WineFsrMode::Performance,
            WineFsrMode::Balanced,
            WineFsrMode::Quality,
            WineFsrMode::Ultra,
        ]
        .map(|m| {
            (
                bigame_core::game_launch::wine_fsr_mode_word(m).to_owned(),
                wine_fsr_mode_name(m),
            )
        }),
    );
    let picker = Picker::new(
        &i18n("Wine FSR quality"),
        "",
        &items,
        value.map_or(INHERIT, bigame_core::game_launch::wine_fsr_mode_word),
    );
    // The sizes for this screen, once it is read.
    let main_screen: Rc<Cell<Option<(u32, u32)>>> = Rc::new(Cell::new(None));
    {
        let main_screen = Rc::clone(&main_screen);
        glib::spawn_future_local(async move {
            let size = gio::spawn_blocking(bigame_core::screen::primary_size)
                .await
                .ok()
                .flatten();
            main_screen.set(size);
        });
    }
    picker.row.add_suffix(&info::dialog_button(
        &i18n("About Wine FSR's quality modes"),
        move || wine_fsr_info(main_screen.get()),
    ));
    picker
}

/// What each Wine FSR mode does, with the sizes for `main` (the main
/// screen) when it is known.
fn wine_fsr_info(main: Option<(u32, u32)>) -> (String, String, Vec<Entry>) {
    // FSR 1's scale factors.
    let modes = [
        (
            i18n("Ultra"),
            1.3,
            i18n("The sharpest image, and the smallest gain."),
        ),
        (
            i18n("Quality"),
            1.5,
            i18n("Close to the full image, with a clear gain."),
        ),
        (i18n("Balanced"), 1.7, i18n("Softer, with more frames.")),
        (
            i18n("Performance"),
            2.0,
            i18n("The most frames; the image is visibly softer."),
        ),
    ];
    let entries = modes
        .into_iter()
        .map(|(name, factor, what)| {
            let size = main.map(|(w, h)| {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss
                )]
                let scaled = |n: u32| (f64::from(n) / factor).round() as u32;
                i18n("On your main screen: about %s.")
                    .replace("%s", &format!("{} × {}", scaled(w), scaled(h)))
            });
            let renders = i18n("The game renders at the screen's size divided by %s.")
                .replace("%s", &factor.to_string());
            Entry {
                title: format!("{name} (÷{factor})"),
                body: match size {
                    Some(size) => format!("{renders} {what} {size}"),
                    None => format!("{renders} {what}"),
                },
            }
        })
        .collect();
    (
        i18n("Wine FSR quality modes"),
        i18n(
            "Wine FSR enlarges a game that runs in exclusive fullscreen at a size below the screen's, with AMD FSR 1. The mode says how much smaller the game renders: the game's list of resolutions gains that size, and choosing it in the game's own video settings turns the upscaling on. It exists only in Proton builds that carry Wine's fullscreen patch, such as GE-Proton and Proton-tkg; Valve's current Proton ignores it.",
        ),
        entries,
    )
}

// ── Looks ───────────────────────────────────────────────────────────────────

/// The ⓘ for vkBasalt's looks.
#[must_use]
pub fn vkbasalt_looks_button() -> gtk4::Button {
    use bigame_core::vkbasalt as vkb;
    info::dialog_button(&i18n("About vkBasalt's looks"), || {
        (
            i18n("vkBasalt's looks"),
            i18n("vkBasalt draws its effects over the finished image of every Vulkan game (and Proton games, through DXVK and VKD3D). It reads the file below."),
            vec![
                Entry {
                    title: i18n("My own file"),
                    body: i18n("The file as you left it, or vkBasalt's example when there is none. Choosing it after a look puts your file back."),
                },
                Entry {
                    title: i18n("CAS sharpening"),
                    body: i18n("AMD's Contrast Adaptive Sharpening, built into vkBasalt: a crisper image, especially with TAA or upscaling, at almost no cost. Nothing is downloaded."),
                },
                Entry {
                    title: i18n("Nara Linux (ReShade)"),
                    body: i18n("Richer colour (Colourfulness), more contrast in light and shade (FakeHDR) and a fine film grain (FilmGrain2). Made by Narayan, of the Nara Linux channel on YouTube: %s. It uses three ReShade shaders, downloaded once from GitHub (ReShade and SweetFX, fixed versions, each checked against its SHA-256) into ~/.local/share/reshade.").replace("%s", vkb::NARA_VIDEO),
                },
            ],
        )
    })
}

/// The look vkBasalt's file holds now, by name.
#[must_use]
pub fn vkbasalt_look_name() -> String {
    use bigame_core::vkbasalt::{Style, StyleState, current_style};
    match current_style() {
        StyleState::Defaults => i18n("vkBasalt's default"),
        StyleState::Own | StyleState::Style(Style::Own) => i18n("My own file"),
        StyleState::Style(Style::Cas) => i18n("CAS sharpening"),
        StyleState::Style(Style::NaraLinux) => i18n("Nara Linux (ReShade)"),
    }
}

/// The ⓘ for `MangoHud`'s overlay styles.
#[must_use]
pub fn mangohud_style_button() -> gtk4::MenuButton {
    info::button(
        &i18n("Overlay style"),
        &i18n(
            "Basic is one line across the top, like the Steam Deck's level 2: frame rate, frame times, CPU and GPU load and power, memory and video memory. Full is a column, like its level 3: the GPU and the CPU each with load, temperature, clock and power, then memory, frame rate and frame times. Battery appears only on a laptop. A per-game MangoHud file (wine-<game>.conf) takes precedence, and a Flatpak launcher reads its own copy.",
        ),
    )
}

/// The overlay style `MangoHud`'s file holds now, by name.
#[must_use]
pub fn mangohud_style_name() -> String {
    use bigame_core::mangohud::{Style, StyleState, current_style};
    match current_style() {
        StyleState::Defaults => i18n("Default"),
        StyleState::Style(Style::Basic) => i18n("Steam Deck — basic"),
        StyleState::Style(Style::Full) => i18n("Steam Deck — full"),
        _ => i18n("My own file"),
    }
}
