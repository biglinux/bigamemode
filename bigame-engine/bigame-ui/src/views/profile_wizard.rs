//! Profile creation wizard: the profile editor, one explained step at a
//! time.
//!
//! Every step shows the very rows the editor shows (`widgets::optimization`)
//! over the same `GameOptimization`, and saving is the same call, so a
//! profile made here opens in the editor exactly as it was made — no value
//! the wizard chose differently, none it left out. A step exists only for
//! what this machine can do; what it cannot do is said in the review
//! instead of being a step with dead controls.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use libadwaita as adw;

use bigame_core::graphics::config::Mode;
use bigame_core::optimization::GameOptimization;
use bigame_core::profiles::GameProfile;

use crate::i18n::{error_text, i18n};
use crate::widgets::game_fields::GameFields;
use crate::widgets::optimization::{self as ui, Machine};

/// Open the wizard dialog attached to `parent`.
pub fn open(parent: &impl IsA<gtk4::Widget>, on_saved: impl Fn(GameProfile) + 'static) {
    open_internal(parent, None, on_saved);
}

/// Open the wizard with a pre-filled suggested program name.
pub fn open_with_suggested_name(
    parent: &impl IsA<gtk4::Widget>,
    suggested_name: &str,
    on_saved: impl Fn(GameProfile) + 'static,
) {
    open_internal(parent, Some(suggested_name.to_owned()), on_saved);
}

/// One step: its page's name in the stack.
struct Step {
    id: &'static str,
}

/// Building a widget tree is inherently linear — splitting it yields helpers
/// with a single caller and no independent meaning — so the length lint is
/// allowed here rather than worked around.
#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
fn open_internal(
    parent: &impl IsA<gtk4::Widget>,
    suggested_name: Option<String>,
    // Moved into the GTK closures that outlive this call.
    on_saved: impl Fn(GameProfile) + 'static,
) {
    let parent_w: gtk4::Widget = parent.clone().upcast();
    let m = Machine::detect();
    let process = suggested_name.clone().unwrap_or_default();
    let draft = GameOptimization::new(&process);
    let game = ui::Game::detect(&process, "", None);
    let fields = Rc::new(GameFields::build(&draft, &m, &game, false));
    let draft = Rc::new(RefCell::new(draft));
    let on_saved = Rc::new(on_saved);

    let dialog = adw::Dialog::builder()
        .title(i18n("Create Profile"))
        .content_width(580)
        .content_height(640)
        .build();

    let stack = gtk4::Stack::new();
    stack.set_transition_duration(200);
    stack.set_vexpand(true);
    let mut steps: Vec<Step> = Vec::new();
    let mut add = |id: &'static str, title: &str, text: &str, content: &gtk4::Widget| {
        stack.add_named(&wizard_step(title, text, content), Some(id));
        steps.push(Step { id });
    };

    // The program falcond recognises the game by.
    let name_entry = adw::EntryRow::builder()
        .title(i18n("Program name (what you see in Task Manager)"))
        .text(&process)
        .build();
    let name_group = adw::PreferencesGroup::new();
    name_group.add_css_class("wizard-input-card");
    name_group.add(&name_entry);
    add(
        "game",
        &i18n("Program Name"),
        &i18n(
            "Enter the exact name of the executable you want to trigger this profile.\nExamples: minecraft, dota2, steam",
        ),
        name_group.upcast_ref(),
    );

    fields.performance.group.add_css_class("wizard-input-card");
    add(
        "perf",
        &i18n("Performance"),
        &i18n(
            "Performance mode keeps the processor at full speed while this game runs, at the cost of more power. The screen can also stay awake while you play.",
        ),
        fields.performance.group.upcast_ref(),
    );
    if fields.scheduler.available() {
        fields.scheduler.group.add_css_class("wizard-input-card");
        add(
            "sched",
            &i18n("CPU scheduler"),
            &i18n(
                "A sched-ext scheduler can smooth frame times in some games. Leave it on the general configuration unless a game stutters; the (i) buttons explain each choice.",
            ),
            fields.scheduler.group.upcast_ref(),
        );
    }
    if fields.vcache.available() {
        fields.vcache.group.add_css_class("wizard-input-card");
        add(
            "vcache",
            &i18n("3D V-Cache"),
            &i18n(
                "Your processor has 3D V-Cache on one of its CCDs. Choose which one this game prefers, or leave it to the general configuration.",
            ),
            fields.vcache.group.upcast_ref(),
        );
    }
    if fields.gamescope.available() {
        fields.gamescope.group.add_css_class("wizard-input-card");
        add(
            "gamescope",
            &i18n("Display Layer"),
            &i18n(
                "Gamescope provides an isolated compositor for the game, enabling resolution scaling, framerate limiting, and FidelityFX Super Resolution (FSR).",
            ),
            fields.gamescope.group.upcast_ref(),
        );
    }
    if fields.frame_generation.available() {
        fields
            .frame_generation
            .group
            .add_css_class("wizard-input-card");
        add(
            "fg",
            &i18n("Frame Generation"),
            &i18n(
                "LSFG-VK inserts synthetically generated frames to multiply your framerate, providing a smoother visual experience at the cost of slight input latency.",
            ),
            fields.frame_generation.group.upcast_ref(),
        );
    }
    if fields.mangohud.available() {
        fields.mangohud.group.add_css_class("wizard-input-card");
        add(
            "mangohud",
            "MangoHud",
            &i18n(
                "The performance overlay: frame rate, frame times, processor and graphics card. How it looks is chosen in Tuning → Monitoring.",
            ),
            fields.mangohud.group.upcast_ref(),
        );
    }

    // AI Graphics: optional, and nothing is applied without its own page.
    let (ai_group, ai_checks) = radio_group(&[
        (
            i18n("Recommended"),
            i18n(
                "BiGame-mode looks at the game and your graphics card and shows what it would do. Nothing changes until you press Apply.",
            ),
        ),
        (
            i18n("Advanced…"),
            i18n("Choose the upscaler and frame generation yourself."),
        ),
        (
            i18n("Not now"),
            i18n("Leave the game's graphics as they are."),
        ),
    ]);
    ai_checks[0].set_active(true);
    ai_group.add_css_class("wizard-input-card");
    add(
        "ai",
        &i18n("AI Graphics"),
        &i18n(
            "Improve image quality and performance using technologies such as DLSS, FSR, XeSS, OptiScaler and compatible neural-rendering features.\nEnable AI Graphics for this game?",
        ),
        ai_group.upcast_ref(),
    );

    // The review is filled just before it is shown.
    let summary_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    add(
        "review",
        &i18n("Profile Summary"),
        &i18n("Review your profile settings before saving."),
        summary_box.upcast_ref(),
    );
    let steps = Rc::new(steps);
    let count = steps.len();

    // ── Progress and navigation ──────────────────────────────────────
    let header = adw::HeaderBar::new();
    header.add_css_class("flat");
    header.set_show_title(false);
    let dots_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    dots_row.set_halign(gtk4::Align::Center);
    dots_row.set_valign(gtk4::Align::Center);
    let dots: Rc<Vec<gtk4::Box>> = Rc::new(
        (0..count)
            .map(|_| {
                let d = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
                d.add_css_class("progress-dot");
                d.set_valign(gtk4::Align::Center);
                dots_row.append(&d);
                d
            })
            .collect(),
    );
    let progress = gtk4::Label::new(None);
    progress.add_css_class("dim-label");
    progress.add_css_class("caption");
    let center = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    center.set_valign(gtk4::Align::Center);
    center.append(&dots_row);
    center.append(&progress);

    let back_btn = gtk4::Button::builder()
        .label(i18n("Back"))
        .css_classes(["flat"])
        .visible(false)
        .build();
    let next_btn = gtk4::Button::builder()
        .label(i18n("Next"))
        .css_classes(["suggested-action", "pill"])
        .build();
    let nav_bar = gtk4::CenterBox::new();
    nav_bar.set_margin_top(12);
    nav_bar.set_margin_bottom(20);
    nav_bar.set_margin_start(20);
    nav_bar.set_margin_end(20);
    nav_bar.set_start_widget(Some(&back_btn));
    nav_bar.set_center_widget(Some(&center));
    nav_bar.set_end_widget(Some(&next_btn));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    toolbar.add_bottom_bar(&nav_bar);
    dialog.set_child(Some(&toolbar));

    let current = Rc::new(std::cell::Cell::new(0usize));
    let show = {
        let (stack, steps, dots) = (stack.clone(), Rc::clone(&steps), Rc::clone(&dots));
        let (back_btn, next_btn, progress) = (back_btn.clone(), next_btn.clone(), progress.clone());
        let (summary_box, fields, draft, name_entry) = (
            summary_box.clone(),
            Rc::clone(&fields),
            Rc::clone(&draft),
            name_entry.clone(),
        );
        let m = m.clone();
        let ai_checks = ai_checks.clone();
        Rc::new(move |step: usize, forward: bool| {
            if step + 1 == count {
                let mut g = draft.borrow().clone();
                name_entry.text().trim().clone_into(&mut g.profile.name);
                fields.apply(&mut g);
                populate_summary(&summary_box, &g, &m, ai_choice(&ai_checks));
            }
            stack.set_transition_type(if forward {
                gtk4::StackTransitionType::SlideLeft
            } else {
                gtk4::StackTransitionType::SlideRight
            });
            stack.set_visible_child_name(steps[step].id);
            for (i, dot) in dots.iter().enumerate() {
                dot.remove_css_class("active");
                dot.remove_css_class("completed");
                if i < step {
                    dot.add_css_class("completed");
                } else if i == step {
                    dot.add_css_class("active");
                }
            }
            progress.set_label(
                &i18n("Step %n of %t")
                    .replace("%n", &(step + 1).to_string())
                    .replace("%t", &count.to_string()),
            );
            back_btn.set_visible(step > 0);
            next_btn.set_label(&if step + 1 == count {
                i18n("Create Profile")
            } else {
                i18n("Next")
            });
        })
    };
    show(0, true);

    {
        let (current, show) = (Rc::clone(&current), Rc::clone(&show));
        back_btn.connect_clicked(move |_| {
            let step = current.get();
            if step > 0 {
                current.set(step - 1);
                show(step - 1, false);
            }
        });
    }
    {
        let (current, show) = (Rc::clone(&current), Rc::clone(&show));
        let dialog = dialog.clone();
        next_btn.connect_clicked(move |btn| {
            let step = current.get();
            if step == 0 && name_entry.text().trim().is_empty() {
                crate::widgets::toast::show(btn, &i18n("Game name cannot be empty"));
                name_entry.grab_focus();
                return;
            }
            if step + 1 < count {
                current.set(step + 1);
                show(step + 1, true);
                return;
            }
            // ── Create: the editor's own save ───────────────────────
            let mut g = draft.borrow().clone();
            name_entry.text().trim().clone_into(&mut g.profile.name);
            fields.apply(&mut g);
            let (errors, _) = g.problems();
            if !errors.is_empty() {
                let errors: Vec<String> = errors.iter().map(|e| i18n(e)).collect();
                crate::widgets::toast::error(
                    btn,
                    &i18n("The profile was not saved"),
                    &errors.join("\n"),
                );
                return;
            }
            btn.set_sensitive(false);
            btn.set_label(&i18n("Saving…"));
            let ai = ai_choice(&ai_checks);
            let (btn, dialog, on_saved, parent_w) = (
                btn.clone(),
                dialog.clone(),
                Rc::clone(&on_saved),
                parent_w.clone(),
            );
            gtk4::glib::spawn_future_local(async move {
                tracing::info!(profile = %g.profile.name, "wizard save requested");
                // Off the main thread: the helper may wait on a Polkit
                // password prompt.
                let saved = gtk4::gio::spawn_blocking(move || {
                    let mut g = g;
                    g.save().map(|report| (g, report))
                })
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("save panicked")));
                match saved {
                    Ok((g, report)) => {
                        tracing::info!(profile = %g.profile.name, "wizard save succeeded");
                        dialog.close();
                        on_saved(g.profile.clone());
                        ui::report_save(&parent_w, &report);
                        if ai != Mode::Off {
                            let process = g.profile.name.clone();
                            let found = gtk4::gio::spawn_blocking(move || {
                                bigame_core::graphics::target_for_process(&process)
                            })
                            .await
                            .ok()
                            .flatten();
                            match found {
                                Some(target) => {
                                    crate::views::ai_graphics::open(&parent_w, target, Some(ai));
                                }
                                None => crate::widgets::toast::show(
                                    &parent_w,
                                    &i18n("AI Graphics needs the game's install folder, and no installed game runs as this program"),
                                ),
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %format!("{e:#}"), "wizard profile not saved");
                        crate::widgets::toast::error(
                            &btn,
                            &i18n("The profile was not saved"),
                            &error_text(&e),
                        );
                        btn.set_sensitive(true);
                        btn.set_label(&i18n("Create Profile"));
                    }
                }
            });
        });
    }

    dialog.present(Some(parent));
}

/// The AI Graphics choice of the step.
fn ai_choice(checks: &[gtk4::CheckButton]) -> Mode {
    if checks[0].is_active() {
        Mode::Recommended
    } else if checks[1].is_active() {
        Mode::Advanced
    } else {
        Mode::Off
    }
}

/// A step: a title, what it is about, and its rows, scrollable.
fn wizard_step(title: &str, description: &str, input: &gtk4::Widget) -> gtk4::ScrolledWindow {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    let title_lbl = gtk4::Label::new(Some(title));
    title_lbl.set_halign(gtk4::Align::Center);
    title_lbl.set_wrap(true);
    title_lbl.set_justify(gtk4::Justification::Center);
    title_lbl.add_css_class("title-2");
    title_lbl.add_css_class("wizard-step-title");
    vbox.append(&title_lbl);

    let desc_lbl = gtk4::Label::new(Some(description));
    desc_lbl.set_halign(gtk4::Align::Center);
    desc_lbl.set_justify(gtk4::Justification::Center);
    desc_lbl.set_wrap(true);
    desc_lbl.set_wrap_mode(gtk4::pango::WrapMode::Word);
    desc_lbl.set_max_width_chars(60);
    desc_lbl.add_css_class("body");
    desc_lbl.add_css_class("dim-label");
    desc_lbl.add_css_class("wizard-step-desc");
    vbox.append(&desc_lbl);

    let clamp = adw::Clamp::builder()
        .maximum_size(480)
        .tightening_threshold(360)
        .child(input)
        .build();
    vbox.append(&clamp);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_propagate_natural_height(true);
    scroll.set_child(Some(&vbox));
    scroll
}

/// Rows of choices, one of which is picked.
fn radio_group(choices: &[(String, String)]) -> (adw::PreferencesGroup, Vec<gtk4::CheckButton>) {
    let group = adw::PreferencesGroup::new();
    let mut checks: Vec<gtk4::CheckButton> = Vec::new();
    for (label, sublabel) in choices {
        let check = match checks.first() {
            None => gtk4::CheckButton::new(),
            Some(first) => gtk4::CheckButton::builder().group(first).build(),
        };
        check.set_valign(gtk4::Align::Center);
        let row = adw::ActionRow::builder()
            .title(label)
            .subtitle(sublabel)
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        group.add(&row);
        checks.push(check);
    }
    (group, checks)
}

/// What the profile will be, line by line, including what this machine
/// cannot do.
fn populate_summary(container: &gtk4::Box, g: &GameOptimization, m: &Machine, ai: Mode) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let group = adw::PreferencesGroup::new();
    group.add_css_class("wizard-input-card");
    let mut rows = ui::summary(g, m);
    rows.push((
        i18n("AI Graphics"),
        match ai {
            Mode::Recommended => i18n("Recommended — opens after the profile is created"),
            Mode::Advanced => i18n("Advanced — opens after the profile is created"),
            Mode::Off => i18n("Not now"),
        },
    ));
    for (key, value) in rows {
        let row = adw::ActionRow::builder()
            .title(&key)
            .subtitle(&value)
            .use_markup(false)
            .build();
        row.set_subtitle_selectable(true);
        group.add(&row);
    }
    container.append(&group);
}
