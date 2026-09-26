//! The video pipeline: what stands between the game and the display, and
//! which of it is really in the running game.
//!
//! Configuration and execution are different states here. *Configured*
//! comes from the settings; *detected* from the game's process — its
//! environment, the libraries it mapped, its process tree, `OptiScaler`'s
//! log. A feature configured but not detected is a problem with a reason
//! and a fix, never "active".

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::overview::{Snapshot, State};

use crate::i18n::i18n;
use crate::widgets::status::{self, Body, Chip, StatusRow};

/// One stage of the pipeline strip.
#[derive(Clone)]
struct Stage {
    root: gtk4::Box,
    mark: gtk4::Image,
}

/// The pipeline group.
#[derive(Clone)]
pub struct Pipeline {
    /// The running game and the strip: a group of their own, since a
    /// preferences group places anything that is not a row below its rows.
    header: adw::PreferencesGroup,
    group: adw::PreferencesGroup,
    game_card: gtk4::Box,
    game_title: gtk4::Label,
    game_facts: gtk4::Label,
    game_chip: Chip,
    stages: Vec<(&'static str, Stage)>,
    gamescope: StatusRow,
    wine_fsr: StatusRow,
    vkbasalt: StatusRow,
    framegen: StatusRow,
    mangohud: StatusRow,
    ai: StatusRow,
    /// Rows of games with AI Graphics installed, inside the AI row.
    ai_installs: Rc<RefCell<Vec<gtk4::Widget>>>,
    /// The last snapshot, for the AI row's body to be rebuilt with the
    /// installs.
    last: Rc<RefCell<Option<Snapshot>>>,
}

const STAGES: &[(&str, &str, &str)] = &[
    ("game", "Game", "applications-games-symbolic"),
    ("runtime", "Proton / Wine", "package-x-generic-symbolic"),
    ("gamescope", "Gamescope", "video-display-symbolic"),
    ("upscaling", "Upscaling", "zoom-in-symbolic"),
    ("vkbasalt", "vkBasalt", "image-x-generic-symbolic"),
    (
        "framegen",
        "Frame generation",
        "media-skip-forward-symbolic",
    ),
    ("display", "Display", "computer-symbolic"),
];

impl Pipeline {
    /// Build the group.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn new() -> Self {
        let header = adw::PreferencesGroup::new();
        header.set_title(&i18n("Video pipeline"));
        header.set_description(Some(&i18n(
            "What sits between the game and the screen. Configured is what you asked for; detected is what the running game shows.",
        )));
        let group = adw::PreferencesGroup::new();

        // The running game.
        let game_title = gtk4::Label::builder()
            .css_classes(["heading"])
            .xalign(0.0)
            .wrap(true)
            .build();
        let game_facts = gtk4::Label::builder()
            .css_classes(["caption", "dim-label"])
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        let game_chip = Chip::new(State::Waiting);
        let text = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.append(&game_title);
        text.append(&game_facts);
        let game_card = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        game_card.add_css_class("card");
        game_card.add_css_class("running-game-card");
        let icon = gtk4::Image::from_icon_name("applications-games-symbolic");
        icon.set_pixel_size(24);
        icon.add_css_class("dim-label");
        icon.set_valign(gtk4::Align::Center);
        game_card.append(&icon);
        game_card.append(&text);
        game_card.append(game_chip.widget());

        // The strip.
        let strip = gtk4::FlowBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .homogeneous(true)
            .column_spacing(4)
            .row_spacing(4)
            .min_children_per_line(4)
            .max_children_per_line(7)
            .css_classes(["pipeline-strip"])
            .build();
        let mut stages = Vec::new();
        for (id, name, icon_name) in STAGES {
            let icon = gtk4::Image::from_icon_name(icon_name);
            icon.set_pixel_size(20);
            let label = gtk4::Label::builder()
                .label(i18n(name))
                .css_classes(["caption"])
                .wrap(true)
                .justify(gtk4::Justification::Center)
                .build();
            let mark = gtk4::Image::from_icon_name("radio-symbolic");
            mark.set_pixel_size(12);
            let root = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
            root.add_css_class("pipeline-stage");
            root.set_halign(gtk4::Align::Fill);
            root.append(&icon);
            root.append(&label);
            root.append(&mark);
            strip.insert(&root, -1);
            stages.push((*id, Stage { root, mark }));
        }

        let column = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        column.append(&game_card);
        column.append(&strip);
        header.add(&column);

        let gamescope = StatusRow::new(
            "Gamescope",
            &i18n("A micro-compositor around the game: scaling, frame limit, a stable fullscreen"),
            "video-display-symbolic",
        );
        let wine_fsr = StatusRow::new(
            "Wine FSR",
            &i18n("Wine's own upscaling for Proton games in exclusive fullscreen"),
            "zoom-in-symbolic",
        );
        let vkbasalt = StatusRow::new(
            "vkBasalt",
            &i18n("Post-processing filters (sharpening, colour) in Vulkan games"),
            "image-x-generic-symbolic",
        );
        let framegen = StatusRow::new(
            &i18n("Frame generation (lsfg-vk)"),
            &i18n("Extra frames between rendered ones; smoother, with more latency"),
            "media-skip-forward-symbolic",
        );
        let mangohud = StatusRow::new(
            "MangoHud",
            &i18n("The performance overlay, chosen per game in Profiles"),
            "utilities-system-monitor-symbolic",
        );
        let ai = StatusRow::new(
            &i18n("AI Graphics (OptiScaler)"),
            &i18n("Upscaling and frame generation inside the game, per game in Profiles"),
            "starred-symbolic",
        );
        for r in [&gamescope, &wine_fsr, &vkbasalt, &framegen, &mangohud, &ai] {
            group.add(r.widget());
        }

        Self {
            header,
            group,
            game_card,
            game_title,
            game_facts,
            game_chip,
            stages,
            gamescope,
            wine_fsr,
            vkbasalt,
            framegen,
            mangohud,
            ai,
            ai_installs: Rc::new(RefCell::new(Vec::new())),
            last: Rc::new(RefCell::new(None)),
        }
    }

    /// The running game and the strip.
    #[must_use]
    pub fn header_group(&self) -> &adw::PreferencesGroup {
        &self.header
    }

    /// The stage rows.
    #[must_use]
    pub fn group(&self) -> &adw::PreferencesGroup {
        &self.group
    }

    fn stage(&self, id: &str, state: State) {
        let Some((_, s)) = self.stages.iter().find(|(k, _)| *k == id) else {
            return;
        };
        s.mark.set_icon_name(Some(status::icon(state)));
        for c in [
            "state-active",
            "state-waiting",
            "state-attention",
            "state-error",
            "state-neutral",
        ] {
            s.root.remove_css_class(c);
        }
        s.root.add_css_class(status::css(state));
        s.root.set_tooltip_text(Some(&status::label(state)));
    }

    /// Show a reading.
    #[allow(clippy::too_many_lines)]
    pub fn show(&self, snap: &Snapshot) {
        *self.last.borrow_mut() = Some(snap.clone());
        let game = snap.game.is_some();

        // ── The running game ────────────────────────────────────────────
        if let Some(g) = &snap.game {
            self.game_title.set_label(&g.display_name);
            let mut facts = vec![
                g.process_name.clone(),
                format!("PID {}", g.pid),
                match &g.runtime {
                    bigame_core::running::Runtime::Native => i18n("Native"),
                    bigame_core::running::Runtime::Proton(t) if !t.is_empty() => t.clone(),
                    bigame_core::running::Runtime::Proton(_) => "Proton".into(),
                    bigame_core::running::Runtime::Wine => "Wine".into(),
                },
            ];
            if g.graphics != bigame_core::running::Graphics::Unknown {
                facts.push(g.graphics.label().to_owned());
            }
            if let Some(card) = &g.render_card {
                facts.push(i18n("renders on %s").replace("%s", card));
            }
            if let Some(secs) = bigame_core::running::running_for(g.pid) {
                facts.push(format!(
                    "{:02}:{:02}:{:02}",
                    secs / 3600,
                    (secs / 60) % 60,
                    secs % 60
                ));
            }
            self.game_facts.set_label(&facts.join(" · "));
            self.game_chip.set(State::Active, Some(&i18n("Running")));
        } else {
            self.game_title.set_label(&i18n("No game running"));
            self.game_facts.set_label(&i18n(
                "Start a game; what it really gets is read from its process and shown here.",
            ));
            self.game_chip.set(State::Waiting, None);
        }
        self.game_card.set_visible(true);

        // ── The strip ───────────────────────────────────────────────────
        self.stage("game", if game { State::Active } else { State::Waiting });
        self.stage(
            "runtime",
            match snap.game.as_ref().map(|g| &g.runtime) {
                Some(bigame_core::running::Runtime::Native) => State::Off,
                Some(_) => State::Active,
                None => State::Waiting,
            },
        );
        self.stage("gamescope", snap.gamescope_state());
        let (up_state, _) = super::overview::upscaling_summary(snap);
        self.stage("upscaling", up_state);
        self.stage("vkbasalt", snap.vkbasalt_state());
        let (fg_state, _) = super::overview::frame_generation_summary(snap);
        self.stage("framegen", fg_state);
        self.stage("display", if game { State::Active } else { State::Waiting });

        let yes = i18n("Yes");
        let no = i18n("No");
        let dash = "—";
        let detected = |d: Option<bool>| match d {
            Some(true) => yes.clone(),
            Some(false) => no.clone(),
            None => dash.to_owned(),
        };

        // ── Gamescope ───────────────────────────────────────────────────
        let gs = snap.gamescope_state();
        let configured = match snap.gamescope_mode {
            bigame_core::gamescope::Mode::Enabled => i18n("Yes, by the game's profile (Always)"),
            bigame_core::gamescope::Mode::Disabled => i18n("No, by the game's profile (Never)"),
            bigame_core::gamescope::Mode::Auto if snap.video.upscaling.gamescope_enabled => {
                i18n("Yes, by Tuning")
            }
            bigame_core::gamescope::Mode::Auto => no.clone(),
        };
        self.gamescope.set_state(
            gs,
            None,
            &stage_line(gs, &i18n("in the game's process tree")),
        );
        let mut body = Body::new()
            .fact(&i18n("Configured"), &configured)
            .fact(
                &i18n("Detected in the game"),
                &detected(snap.in_game.as_ref().map(|g| g.gamescope)),
            )
            .fact(
                &i18n("Installed"),
                if snap.gamescope_installed { &yes } else { &no },
            );
        if snap.video.upscaling.gamescope_enabled && snap.video.upscaling.base_width > 0 {
            body = body.fact(
                &i18n("Render size"),
                &format!(
                    "{}×{} → {}",
                    snap.video.upscaling.base_width,
                    snap.video.upscaling.base_height,
                    match snap.video.upscaling.gamescope_filter {
                        bigame_core::models::GamescopeFilter::Fsr => "FSR",
                        bigame_core::models::GamescopeFilter::Nis => "NIS",
                        bigame_core::models::GamescopeFilter::Integer => "integer",
                    }
                ),
            );
        }
        body = match gs {
            State::NotDetected => body
                .note(&i18n("Expected: the game inside a gamescope process. Found: no Gamescope in its process tree."))
                .note(&i18n("Likely reasons: the game was started from Steam or its launcher, not from BiGame-mode (Profiles → Launch); Gamescope ended at start (its WSI layer, a game that prefers Wayland); the game's profile says Never."))
                .note(&i18n("Start the game from Profiles → ⋮ → Launch (Turbo). A Steam game is started by Steam, in its own process tree: Gamescope reaches it only through Steam's launch options.")),
            State::Missing => body.command("sudo pacman -S gamescope"),
            _ => body,
        };
        body = body.note(&i18n("Evidence: the parent processes of the game."));
        self.gamescope.set_body(body.build());

        // ── Wine FSR ────────────────────────────────────────────────────
        let wf = snap.wine_fsr_state();
        self.wine_fsr.set_state(
            wf,
            None,
            &stage_line(wf, &i18n("in the game's environment")),
        );
        let mut body = Body::new()
            .fact(
                &i18n("Configured"),
                if snap.video.upscaling.wine_fsr_enabled {
                    &yes
                } else {
                    &no
                },
            )
            .fact(&i18n("Variable"), "WINE_FULLSCREEN_FSR=1")
            .fact(
                &i18n("Detected in the game"),
                &detected(snap.wine_fsr_in_game),
            );
        if wf == State::NotDetected {
            body = body.note(&i18n(
                "The variable is written to ~/.config/environment.d and pushed into the session, but a launcher that was already running (Steam) keeps its old environment: close and reopen Steam, then start the game again.",
            ));
        }
        if snap.ai_graphics.is_some() {
            body = body.note(&i18n(
                "This game has AI Graphics installed: Wine FSR is turned off for its launch, so two upscalers never run in series.",
            ));
        }
        body = body.note(&i18n(
            "It only takes effect in exclusive fullscreen, at a resolution below the desktop's. Evidence: the game process's environment.",
        ));
        self.wine_fsr.set_body(body.build());

        // ── vkBasalt ────────────────────────────────────────────────────
        let vb = snap.vkbasalt_state();
        self.vkbasalt
            .set_state(vb, None, &stage_line(vb, &i18n("loaded in the game")));
        let mut body = Body::new()
            .fact(
                &i18n("Configured"),
                if snap.video.upscaling.vkbasalt_enabled {
                    &yes
                } else {
                    &no
                },
            )
            .fact(
                &i18n("Layer installed"),
                if snap.vkbasalt_installed { &yes } else { &no },
            )
            .fact(
                &i18n("Detected in the game"),
                &detected(snap.in_game.as_ref().map(|g| g.vkbasalt)),
            );
        body = match vb {
            State::NotDetected => body.note(&i18n(
                "vkBasalt is a Vulkan layer: it loads only in Vulkan (and Proton) games, and only when ENABLE_VKBASALT=1 reached the game. A launcher started before the setting keeps its old environment; restart it.",
            )),
            State::Missing => body.command("sudo pacman -S vkbasalt"),
            _ => body,
        };
        body = body.note(&i18n("A look, not a speed-up: it costs a little GPU time. Evidence: libvkbasalt mapped in the game."));
        self.vkbasalt.set_body(body.build());

        // ── Frame generation ────────────────────────────────────────────
        let fg = snap.frame_generation_state();
        let multiplier = snap.in_game.as_ref().and_then(|g| g.frame_generation);
        let chip = multiplier.map(|m| format!("×{m}"));
        self.framegen.set_state(
            fg,
            chip.as_deref(),
            &stage_line(fg, &i18n("generating in the game")),
        );
        let mut body = Body::new()
            .fact(
                &i18n("Global switch (Tuning)"),
                if snap.lsfg.global_on { &yes } else { &no },
            )
            .fact(
                &i18n("For this game"),
                &match snap.lsfg.game_multiplier {
                    Some(m) => format!("×{m}"),
                    None if game => i18n("no entry (off)"),
                    None => dash.to_owned(),
                },
            )
            .fact(
                &i18n("Layer installed"),
                if snap.lsfg.installed { &yes } else { &no },
            )
            .fact(
                &i18n("Lossless.dll"),
                &if snap.lsfg.dll_ready {
                    i18n("found")
                } else {
                    i18n("not configured")
                },
            )
            .fact(
                &i18n("Detected in the game"),
                &detected(snap.in_game.as_ref().map(|g| g.frame_generation.is_some())),
            );
        if snap
            .in_game
            .as_ref()
            .is_some_and(|g| g.frame_generation_changed)
        {
            body = body.note(&i18n(
                "lsfg-vk's file changed after the game started. It takes a new multiplier live, but turning generation on or off waits for the game's next start.",
            ));
        }
        body = match fg {
            State::Missing if !snap.lsfg.installed => body
                .note(&i18n("lsfg-vk is not installed."))
                .command("sudo pacman -S lsfg-vk"),
            State::Missing => body.note(&i18n(
                "lsfg-vk needs your own Lossless.dll (from Lossless Scaling on Steam). Set its path in Tuning → Frame generation.",
            )),
            State::NotDetected => body.note(&i18n(
                "The layer loads only in Vulkan (and Proton) games, and generates only for a game that started with an entry. Start the game again.",
            )),
            _ => body,
        };
        if snap.ai_frame_generation {
            body = body.note(&i18n("OptiScaler generates this game's frames, so lsfg-vk is turned off for its launch: two frame generators never run in series."));
        }
        body = body.note(&i18n("Raises the presented frame rate, not the rendered one, and adds latency. Evidence: liblsfg-vk mapped in the game, and lsfg-vk's entry for it."));
        self.framegen.set_body(body.build());

        // ── MangoHud ────────────────────────────────────────────────────
        let mh = snap.mangohud_state();
        self.mangohud
            .set_state(mh, None, &stage_line(mh, &i18n("loaded in the game")));
        let mode = match snap.mangohud_for_game {
            bigame_core::mangohud::Mode::Off => i18n("Off"),
            bigame_core::mangohud::Mode::On => i18n("On (Vulkan layer)"),
            bigame_core::mangohud::Mode::Forced => i18n("Forced (wrapper)"),
        };
        let mut body = Body::new()
            .fact(
                &i18n("Configured for this game"),
                if game { &mode } else { dash },
            )
            .fact(
                &i18n("Installed"),
                if snap.mangohud_installed { &yes } else { &no },
            )
            .fact(
                &i18n("Detected in the game"),
                &detected(snap.in_game.as_ref().map(|g| g.mangohud)),
            );
        body = match mh {
            State::Missing => body.command("sudo pacman -S mangohud"),
            State::NotDetected => body.note(&i18n(
                "For a Steam game the choice is written into Steam's launch options, with Steam closed; Steam started before the change keeps the old options.",
            )),
            _ => body,
        };
        self.mangohud.set_body(body.build());

        // ── AI Graphics ─────────────────────────────────────────────────
        self.show_ai(snap);
    }

    /// The AI Graphics row: the running game first, then every game with
    /// files installed.
    fn show_ai(&self, snap: &Snapshot) {
        use bigame_core::graphics::runtime::Status;
        let game = snap.game.is_some();
        let (state, line) = match &snap.ai_graphics {
            Some(st @ Status::Active { .. }) => {
                (State::Active, crate::views::ai_graphics::status_text(st))
            }
            Some(Status::NotInstalled) | None if game => {
                (State::Off, i18n("Nothing installed for the running game"))
            }
            Some(st) => (
                State::NotDetected,
                crate::views::ai_graphics::status_text(st),
            ),
            None => (
                State::Waiting,
                i18n("Per game, from the game's card in Profiles"),
            ),
        };
        // Not asked for is not a problem: a game without AI Graphics is off.
        let state =
            if state == State::NotDetected && matches!(snap.ai_graphics, Some(Status::Starting)) {
                State::Configured
            } else {
                state
            };
        self.ai.set_state(state, None, &line);
        let mut body = Body::new().note(&i18n(
            "OptiScaler runs FSR, XeSS or DLSS in place of the upscaler the game already has, and can generate frames. It is installed per game, with every replaced file backed up.",
        ));
        if let Some(st) = &snap.ai_graphics {
            body = body.fact(
                &i18n("Running game"),
                &crate::views::ai_graphics::status_text(st),
            );
        }
        if matches!(snap.ai_graphics, Some(Status::Loaded { .. })) {
            body = body.note(&i18n(
                "OptiScaler loaded but created no upscaler: the game's own upscaler that it takes over is off. Choose it in the game's graphics menu (the plan names it), or open the game's AI Graphics and apply again.",
            ));
        }
        body = body.note(&i18n("Evidence: OptiScaler's log in the game folder, written since the process started, and the DLLs the game mapped."));
        let mut rows = body.build();
        rows.extend(self.ai_installs.borrow().iter().cloned());
        self.ai.set_body(rows);
    }

    /// Re-read which games have AI Graphics installed, and what each is
    /// doing now. Hashes files, so it runs off the main thread and rarely.
    pub fn refresh_installs(&self) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(|| {
                bigame_core::graphics::installed()
                    .into_iter()
                    .map(|t| {
                        let status = bigame_core::graphics::status(&t);
                        let version = bigame_core::graphics::manifest::Manifest::load(
                            &bigame_core::graphics::state_dir(),
                            &t.key(),
                        )
                        .ok()
                        .flatten()
                        .map(|m| m.source.version);
                        (t, status, version)
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let mut rows: Vec<gtk4::Widget> = Vec::new();
            if found.is_empty() {
                rows.push(
                    status::fact_row(&i18n("Games with files installed"), &i18n("none")).upcast(),
                );
            }
            for (target, st, version) in found {
                let status_text = crate::views::ai_graphics::status_text(&st);
                let row = adw::ActionRow::builder()
                    .title(&target.name)
                    .subtitle(match version {
                        Some(v) => format!("OptiScaler {v} · {status_text}"),
                        None => status_text,
                    })
                    .use_markup(false)
                    .build();
                let open = gtk4::Button::builder()
                    .icon_name("go-next-symbolic")
                    .valign(gtk4::Align::Center)
                    .tooltip_text(i18n("Open"))
                    .css_classes(["flat"])
                    .build();
                let t = target.clone();
                open.connect_clicked(move |b| {
                    crate::views::ai_graphics::open(b, t.clone(), None);
                });
                row.add_suffix(&open);
                row.set_activatable_widget(Some(&open));
                rows.push(row.upcast());
            }
            *this.ai_installs.borrow_mut() = rows;
            let last = this.last.borrow().clone();
            if let Some(snap) = last {
                this.show_ai(&snap);
            }
        });
    }
}

/// The one-line subtitle of a stage row, from its state.
fn stage_line(state: State, evidence: &str) -> String {
    match state {
        State::Active => i18n("Working: %s").replace("%s", evidence),
        State::Waiting => i18n("Configured; applies when a game starts from BiGame-mode"),
        State::NotDetected => i18n("Configured, but not %s").replace("%s", evidence),
        State::Configured => i18n("Configured; whether it took cannot be read from outside"),
        State::Off => i18n("Not configured"),
        State::Missing => i18n("Configured, but something it needs is missing"),
        State::Unsupported => i18n("Not supported here"),
        State::Error => i18n("Failed"),
    }
}
