//! Settings: the interface's look, what BiGame-mode does on its own, and how
//! to undo its control.
//!
//! About is not here: the application menu has it.

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use crate::i18n::{i18n, ni18n, tr};
use crate::settings;
use crate::widgets::info;

fn switch(title: &str, subtitle: &str, active: bool, about: &str) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(title)
        .subtitle(subtitle)
        .active(active)
        .build();
    row.add_suffix(&info::button(title, about));
    row
}

/// A segmented control: one button per choice, exactly one pressed.
fn toggles(choices: &[(&str, String)], active: &str) -> adw::ToggleGroup {
    let group = adw::ToggleGroup::builder()
        .valign(gtk4::Align::Center)
        .homogeneous(true)
        .build();
    for (name, label) in choices {
        group.add(adw::Toggle::builder().name(*name).label(label).build());
    }
    group.set_active_name(Some(active));
    group
}

/// The look of the interface: design and colour scheme, applied at once.
fn appearance_group() -> adw::PreferencesGroup {
    use crate::theme::{self, Design, Scheme};

    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Appearance"));
    let (design, scheme) = theme::saved();

    let design_row = adw::ActionRow::builder()
        .title(i18n("Interface theme"))
        .subtitle(if theme::gamer_suspended() {
            i18n("Default is shown while the desktop asks for high contrast")
        } else {
            i18n("Only the look changes: every page works the same")
        })
        .build();
    let design_toggles = toggles(
        &[
            (Design::Default.id(), i18n("Default")),
            (Design::Gamer.id(), i18n("Gamer")),
        ],
        design.id(),
    );
    design_toggles.connect_active_name_notify(|g| {
        if let Some(name) = g.active_name() {
            theme::set_design(Design::from_id(&name));
        }
    });
    design_row.add_suffix(&design_toggles);
    group.add(&design_row);

    let scheme_row = adw::ActionRow::builder()
        .title(i18n("Colour scheme"))
        .subtitle(i18n("System follows the desktop's light or dark setting"))
        .build();
    let scheme_toggles = toggles(
        &[
            (Scheme::System.id(), i18n("System")),
            (Scheme::Light.id(), i18n("Light")),
            (Scheme::Dark.id(), i18n("Dark")),
        ],
        scheme.id(),
    );
    scheme_toggles.connect_active_name_notify(|g| {
        if let Some(name) = g.active_name() {
            theme::set_scheme(Scheme::from_id(&name));
        }
    });
    scheme_row.add_suffix(&scheme_toggles);
    group.add(&scheme_row);
    group
}

/// Build the Settings page.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let current = settings::load();

    page.add(&appearance_group());

    // ── Turbo Mode ──────────────────────────────────────────────────────
    let turbo = adw::PreferencesGroup::new();
    turbo.set_title(&i18n("Turbo Mode"));

    let login = switch(
        &i18n("Start in the background at login"),
        &i18n("So games you start from Steam are noticed even with this window closed"),
        settings::starts_at_login(),
        &i18n(
            "Adds a login entry for your user only (~/.config/autostart). BiGame-mode then runs in the tray and can offer a profile when a new game starts. Turning this off removes the entry.",
        ),
    );
    login.connect_active_notify(|row| {
        if let Err(e) = settings::set_starts_at_login(row.is_active()) {
            crate::widgets::toast::show(row, &format!("{}: {e}", i18n("Could not change it")));
        }
    });
    turbo.add(&login);

    let owner = adw::ActionRow::builder()
        .title(i18n("Hand falcond back"))
        .subtitle(match bigame_core::turbo::owned_since() {
            Some(t) => i18n("BiGame-mode has managed falcond since %s").replace(
                "%s",
                &glib::DateTime::from_unix_local(i64::try_from(t).unwrap_or(0))
                    .and_then(|d| d.format("%x %X"))
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            ),
            None => i18n("BiGame-mode has not changed falcond's service"),
        })
        .build();
    let release = gtk4::Button::builder()
        .label(i18n("Hand back"))
        .valign(gtk4::Align::Center)
        .sensitive(bigame_core::turbo::owned_since().is_some())
        .build();
    owner.add_suffix(&release);
    owner.add_suffix(&info::button(
        &i18n("Hand falcond back"),
        &i18n(
            "Turbo turns falcond's service on and off. The first time it did, it recorded whether falcond was enabled and running. Handing back restores exactly that and stops Turbo from managing it until you turn Turbo on again.",
        ),
    ));
    release.connect_clicked(|b| {
        let b = b.clone();
        glib::spawn_future_local(async move {
            b.set_sensitive(false);
            let result = gio::spawn_blocking(|| {
                bigame_core::dbus_client::daemon_proxy_blocking()
                    .and_then(|p| Ok(p.release_game_backend()?))
            })
            .await;
            let text = match result {
                Ok(Ok(true)) => i18n("falcond is back as it was before BiGame-mode"),
                Ok(Ok(false)) => i18n("There was nothing to hand back"),
                Ok(Err(e)) => format!("{}: {e:#}", i18n("Could not hand it back")),
                Err(_) => i18n("Could not hand it back"),
            };
            crate::widgets::toast::show(&b, &text);
        });
    });
    turbo.add(&owner);
    page.add(&turbo);

    // ── Game profiles ───────────────────────────────────────────────────
    let profiles = adw::PreferencesGroup::new();
    profiles.set_title(&i18n("Game profiles"));

    let offer = switch(
        &i18n("Offer a profile for new games"),
        &i18n("When Turbo is on and a game without its own profile starts"),
        current.offer_profiles,
        &i18n(
            "Shows a notification, never a window over your game. The profile is built for this machine, and its review shows why each value was chosen. Nothing is created unless you choose to.",
        ),
    );
    offer.connect_active_notify(|row| {
        let mut s = settings::load();
        s.offer_profiles = row.is_active();
        settings::save(&s);
    });
    profiles.add(&offer);

    let migrate = adw::ActionRow::builder()
        .title(i18n("Profiles from an older BiGame-mode"))
        .subtitle(i18n("Checking…"))
        .use_markup(false)
        .build();
    let migrate_button = gtk4::Button::builder()
        .label(i18n("Fix"))
        .valign(gtk4::Align::Center)
        .visible(false)
        .build();
    migrate.add_suffix(&migrate_button);
    migrate.add_suffix(&info::button(
        &i18n("Profiles from an older BiGame-mode"),
        &i18n(
            "Older versions named profiles after the game's title, which falcond can never match, and stored settings falcond ignores. Fixing renames each one to the game's real process and keeps only falcond's settings. Every profile is backed up first to ~/.local/state/bigame-mode. falcond's own profiles are never touched.",
        ),
    ));
    profiles.add(&migrate);
    {
        let migrate = migrate.clone();
        let button = migrate_button.clone();
        glib::spawn_future_local(async move {
            let plan = gio::spawn_blocking(|| {
                bigame_core::migration::plan(
                    std::path::Path::new(bigame_core::profiles::USER_PROFILES_DIR),
                    &bigame_core::games::detect_all(),
                )
            })
            .await
            .unwrap_or_default();
            let fixable = plan
                .iter()
                .filter(|a| {
                    matches!(
                        a,
                        bigame_core::migration::Action::Rekey { .. }
                            | bigame_core::migration::Action::Clean { .. }
                    )
                })
                .count();
            if fixable == 0 {
                migrate.set_subtitle(&i18n("None need fixing"));
                return;
            }
            migrate.set_subtitle(&ni18n(
                "%n can never match its game as it is",
                "%n can never match their game as they are",
                fixable,
            ));
            button.set_visible(true);
            let migrate = migrate.clone();
            button.connect_clicked(move |b| {
                let plan = plan.clone();
                let migrate = migrate.clone();
                let b = b.clone();
                glib::spawn_future_local(async move {
                    b.set_sensitive(false);
                    let result = gio::spawn_blocking(move || {
                        let state = std::env::var_os("HOME")
                            .map(|h| std::path::Path::new(&h).join(".local/state/bigame-mode"))
                            .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
                        bigame_core::migration::apply(
                            &plan,
                            std::path::Path::new(bigame_core::profiles::USER_PROFILES_DIR),
                            &state,
                        )
                    })
                    .await;
                    match result {
                        Ok(Ok((_, done))) => {
                            let done: Vec<String> = done.iter().map(tr).collect();
                            migrate.set_subtitle(&done.join(" · "));
                            b.set_visible(false);
                        }
                        Ok(Err(e)) => {
                            migrate.set_subtitle(&format!("{}: {e:#}", i18n("Could not fix them")));
                            b.set_sensitive(true);
                        }
                        Err(_) => b.set_sensitive(true),
                    }
                });
            });
        });
    }
    page.add(&profiles);

    // ── Notifications ───────────────────────────────────────────────────
    let notif = adw::PreferencesGroup::new();
    notif.set_title(&i18n("Notifications"));
    let notif_row = switch(
        &i18n("Game Notifications"),
        &i18n("Show notifications on game launch and exit"),
        current.notifications_enabled,
        &i18n(
            "Desktop notifications when a game profile is applied and when it is restored. The profile offer is controlled separately above.",
        ),
    );
    notif_row.connect_active_notify(|row| {
        let mut s = settings::load();
        s.notifications_enabled = row.is_active();
        settings::save(&s);
    });
    notif.add(&notif_row);
    page.add(&notif);

    // ── Monitoring ──────────────────────────────────────────────────────
    let monitoring = adw::PreferencesGroup::new();
    monitoring.set_title(&i18n("Monitoring"));
    let ping_row = adw::EntryRow::builder()
        .title(i18n("Ping Target"))
        .text(&current.ping_target)
        .build();
    ping_row.add_suffix(&info::button(
        &i18n("Ping Target"),
        &i18n("The address the Details page pings for its latency graph. It is only contacted while that page is on screen."),
    ));
    ping_row.connect_changed(|row| {
        let text = row.text().to_string();
        if !text.is_empty() {
            let mut s = settings::load();
            s.ping_target = text;
            settings::save(&s);
        }
    });
    monitoring.add(&ping_row);
    page.add(&monitoring);

    page
}
