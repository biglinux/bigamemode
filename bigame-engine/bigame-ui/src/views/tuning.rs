//! Tuning view: scheduler, governor, V-Cache and device settings.
//!
//! Controls are wired to falcond config via `bigame_core::config`. Changes are
//! written through the privileged helper, which reloads falcond.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use crate::i18n::i18n;

/// Shared mutable config state for coordinated writes.
type SharedConfig = Rc<RefCell<bigame_core::config::FalcondConfig>>;

/// Build the Tuning view with all configuration controls.
///
/// Sections: Scheduler, CPU Governor, `VCache`, Device Mode.
/// `AdwPreferencesPage` handles scrolling internally via `AdwClampScrollable`.
#[must_use]
pub fn build() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    // Load current config (best-effort, fallback to defaults)
    let config = bigame_core::config::read().unwrap_or_default();
    let shared = Rc::new(RefCell::new(config));

    page.add(&build_daemon_group(&shared));
    page.add(&build_scheduler_group(&shared));
    page.add(&build_governor_group());
    if let Some(group) = build_vcache_group(&shared) {
        page.add(&group);
    }

    let active_game = bigame_core::status::read()
        .and_then(|s| s.active_profile)
        .unwrap_or_default();
    page.add(&crate::widgets::fg_controls::build_tuning_fg_group(
        &active_game,
    ));

    page.add(&build_device_group(&shared));
    page.add(&build_advanced_group());

    page
}

/// Options an ordinary user should never need, kept out of the way.
///
/// Collapsed by default and deliberately last: a beginner presses Turbo and
/// plays, everything here exists for the person who already knows what a
/// scheduler flag is, and putting it in front of everyone else only makes the
/// page harder to read.
///
/// Nothing here is a hidden setting — each row states what it writes and where.
// One group, built top to bottom; splitting it would scatter the rows.
#[allow(clippy::too_many_lines)]
fn build_advanced_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Advanced"));

    let expander = adw::ExpanderRow::builder()
        .title(i18n("Show advanced options"))
        .subtitle(i18n("Scheduler flags and raw Gamescope arguments"))
        .build();
    group.add(&expander);

    // ── sched-ext flags ─────────────────────────────────────────────────
    let caps = bigame_core::capabilities::Capabilities::detect();
    let scx = &caps.sched_ext;

    let scx_status = adw::ActionRow::builder()
        .title(i18n("sched-ext availability"))
        .subtitle(match scx.switchable().describe() {
            // Saying *why* it cannot be switched leads to a different fix than
            // "unsupported" would: install scx-tools, not new hardware.
            Some(reason) => reason,
            None => i18n("Available — falcond applies the scheduler you configure above"),
        })
        .build();
    scx_status.add_prefix(&gtk4::Image::from_icon_name(
        if scx.switchable().is_available() {
            "object-select-symbolic"
        } else {
            "dialog-warning-symbolic"
        },
    ));
    expander.add_row(&scx_status);

    let installed = adw::ActionRow::builder()
        .title(i18n("Schedulers installed"))
        .subtitle(if scx.installed.is_empty() {
            i18n("None found in /usr/bin")
        } else {
            scx.installed.join(", ")
        })
        .build();
    expander.add_row(&installed);

    // ── Raw Gamescope arguments ─────────────────────────────────────────
    let gamescope_row = adw::ActionRow::builder()
        .title(i18n("Gamescope options this build accepts"))
        .subtitle(match &caps.gamescope {
            Some(gs) => i18n("Version %v — %n options detected from --help")
                .replace(
                    "%v",
                    &gs.version
                        .map_or_else(|| i18n("unknown"), |v| v.to_string()),
                )
                .replace("%n", &gs.flags.len().to_string()),
            None => i18n("Gamescope is not installed"),
        })
        .build();
    expander.add_row(&gamescope_row);

    // Showing the generated command line is the honest version of an "advanced
    // options" box: the arguments are built from capabilities, so the useful
    // thing is to see what they came out as, not to retype them.
    if let Some(gs) = caps.gamescope.as_ref() {
        let sample = bigame_core::gamescope::Config {
            render_width: 1920,
            render_height: 1080,
            filter: bigame_core::gamescope::Filter::Fsr,
            sharpness: 5,
            ..bigame_core::gamescope::Config::default()
        };
        let built = sample.to_args(gs);
        let preview = adw::ActionRow::builder()
            .title(i18n("Example command line"))
            .subtitle(format!(
                "gamescope {} -- &lt;game&gt;",
                built.args.join(" ")
            ))
            .build();
        preview.set_subtitle_selectable(true);
        expander.add_row(&preview);

        for unsupported in &built.unsupported {
            let row = adw::ActionRow::builder()
                .title(i18n("Not supported by this Gamescope"))
                .subtitle(format!("--{} — {}", unsupported.flag, unsupported.effect))
                .build();
            row.add_prefix(&gtk4::Image::from_icon_name("dialog-warning-symbolic"));
            expander.add_row(&row);
        }
    }

    group
}

/// Write the shared config through the privileged helper, on a background thread.
fn save_config(shared: &SharedConfig) {
    let cfg = shared.borrow().clone();
    glib::spawn_future_local(async move {
        if let Err(e) = bigame_core::config::write(&cfg).await {
            tracing::error!("config write failed: {e}");
        }
    });
}

/// Daemon settings: performance mode toggle + poll interval.
fn build_daemon_group(shared: &SharedConfig) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Daemon"));
    group.set_description(Some(&i18n("falcond global behavior")));

    // Performance mode toggle
    let perf_row = adw::SwitchRow::builder()
        .title(i18n("Performance Mode"))
        .subtitle(i18n(
            "Enable performance optimizations when games are detected",
        ))
        .active(shared.borrow().enable_performance_mode)
        .build();
    group.add(&perf_row);

    let cfg = Rc::clone(shared);
    perf_row.connect_active_notify(move |row| {
        cfg.borrow_mut().enable_performance_mode = row.is_active();
        save_config(&cfg);
    });

    // Poll interval (ms)
    let poll_adj = gtk4::Adjustment::new(
        f64::from(shared.borrow().poll_interval_ms),
        500.0,
        60_000.0,
        500.0,
        1000.0,
        0.0,
    );
    let poll_row = adw::SpinRow::new(Some(&poll_adj), 500.0, 0);
    poll_row.set_title(&i18n("Poll Interval (ms)"));
    poll_row.set_subtitle(&i18n(
        "How often falcond looks for a running game. Lower reacts sooner and \
         costs more; the default is 9000.",
    ));
    group.add(&poll_row);

    let cfg = Rc::clone(shared);
    poll_row.connect_changed(move |row| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let val = row.value() as u32;
        cfg.borrow_mut().poll_interval_ms = val;
        save_config(&cfg);
    });

    group
}

/// Scheduler section: sched-ext scheduler + mode selection.
fn build_scheduler_group(shared: &SharedConfig) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Scheduler (sched-ext)"));

    let detected = bigame_core::sched::detect_installed();
    // detected always contains "none"; if len==1, no real schedulers installed.
    let has_schedulers = detected.len() > 1;

    if has_schedulers {
        group.set_description(Some(&i18n(
            "BPF-based process scheduler for gaming workloads",
        )));
    } else {
        group.set_description(Some(&i18n(
            "No sched-ext schedulers found. Install scx-scheds to enable this feature.\n\
             Command: sudo pacman -S scx-scheds",
        )));
    }

    let sched_strs: Vec<&str> = detected.iter().map(String::as_str).collect();
    let sched_model = gtk4::StringList::new(&sched_strs);
    let sched_row = adw::ComboRow::builder()
        .title(i18n("Scheduler"))
        .subtitle(i18n("Active sched-ext scheduler"))
        .model(&sched_model)
        .sensitive(has_schedulers)
        .build();
    sched_row.set_selected(crate::views::profiles::find_index(
        &sched_model,
        &shared.borrow().scx_sched,
    ));

    let info_btn = gtk4::Button::builder()
        .icon_name("dialog-information-symbolic")
        .valign(gtk4::Align::Center)
        .css_classes(["flat", "circular"])
        .tooltip_text(i18n("Learn about Schedulers"))
        .build();
    info_btn.connect_clicked(|btn| {
        if let Some(win) = btn.root().and_downcast::<gtk4::Window>() {
            crate::widgets::scheduler_info::show(&win);
        }
    });
    sched_row.add_suffix(&info_btn);

    group.add(&sched_row);

    let mode_model = gtk4::StringList::new(&["default", "gaming", "power", "latency", "server"]);
    let mode_row = adw::ComboRow::builder()
        .title(i18n("Mode"))
        .subtitle(i18n("Scheduler tuning preset"))
        .model(&mode_model)
        .sensitive(has_schedulers)
        .build();
    mode_row.set_selected(crate::views::profiles::find_index(
        &mode_model,
        &shared.borrow().scx_sched_props,
    ));
    group.add(&mode_row);

    // Connect: scheduler change → update config + save
    let cfg = Rc::clone(shared);
    let sm = sched_model.clone();
    sched_row.connect_selected_notify(move |row| {
        if let Some(val) = sm.string(row.selected()) {
            cfg.borrow_mut().scx_sched = val.to_string();
            save_config(&cfg);
        }
    });

    let cfg = Rc::clone(shared);
    let mm = mode_model.clone();
    mode_row.connect_selected_notify(move |row| {
        if let Some(val) = mm.string(row.selected()) {
            cfg.borrow_mut().scx_sched_props = val.to_string();
            save_config(&cfg);
        }
    });

    group
}

/// CPU governor section (read-only display from sysfs).
fn build_governor_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("CPU Governor"));
    group.set_description(Some(&i18n(
        "Frequency scaling policy (managed by PowerProfiles)",
    )));

    let gov_model = gtk4::StringList::new(&[]);
    let gov_row = adw::ComboRow::builder()
        .title(i18n("Governor"))
        .subtitle(i18n("Applied to all CPU cores"))
        .model(&gov_model)
        .sensitive(false) // read-only: governor is set by PowerProfiles
        .build();
    group.add(&gov_row);

    // Populate from sysfs (background)
    let model = gov_model.clone();
    let row = gov_row.clone();
    glib::spawn_future_local(async move {
        let (available, current) = gio::spawn_blocking(|| {
            let avail = std::fs::read_to_string(
                "/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors",
            )
            .unwrap_or_default();
            let curr =
                std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
                    .unwrap_or_default();
            (avail, curr)
        })
        .await
        .unwrap_or_default();

        let govs: Vec<&str> = available.split_whitespace().collect();
        for g in &govs {
            model.append(g);
        }
        let curr = current.trim();
        for (i, g) in govs.iter().enumerate() {
            if *g == curr {
                #[allow(clippy::cast_possible_truncation)]
                row.set_selected(i as u32);
                break;
            }
        }
    });

    group
}

/// `VCache` mode section (AMD 3D V-Cache), only on a CPU that has one: a
/// disabled control for hardware that is not there still reads as a feature.
/// Diagnostics says when the CPU has none.
fn build_vcache_group(shared: &SharedConfig) -> Option<adw::PreferencesGroup> {
    if !bigame_core::vcache::is_available() {
        return None;
    }
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("VCache (AMD 3D V-Cache)"));
    group.set_description(Some(&i18n(
        "Cache optimization mode for AMD 3D V-Cache CPUs",
    )));
    let model = gtk4::StringList::new(&["none", "cache", "freq"]);
    let row = adw::ComboRow::builder()
        .title(i18n("VCache Mode"))
        .subtitle(i18n("Cache optimization strategy"))
        .model(&model)
        .build();
    row.set_selected(crate::views::profiles::find_index(
        &model,
        &shared.borrow().vcache_mode,
    ));
    group.add(&row);

    let cfg = Rc::clone(shared);
    row.connect_selected_notify(move |row| {
        if let Some(val) = model.string(row.selected()) {
            cfg.borrow_mut().vcache_mode = val.to_string();
            save_config(&cfg);
        }
    });

    Some(group)
}

/// Device mode section (desktop / handheld / HTPC).
fn build_device_group(shared: &SharedConfig) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Device Mode"));
    group.set_description(Some(&i18n(
        "Profile set matching your hardware form factor",
    )));

    let model = gtk4::StringList::new(&["none", "handheld", "htpc"]);
    let row = adw::ComboRow::builder()
        .title(i18n("Mode"))
        .subtitle(i18n("Selects profile directory for game matching"))
        .model(&model)
        .build();
    row.set_selected(crate::views::profiles::find_index(
        &model,
        &shared.borrow().profile_mode,
    ));
    group.add(&row);

    let cfg = Rc::clone(shared);
    row.connect_selected_notify(move |row| {
        if let Some(val) = model.string(row.selected()) {
            cfg.borrow_mut().profile_mode = val.to_string();
            save_config(&cfg);
        }
    });

    group
}
