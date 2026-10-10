//! Settings: the interface's look, what Big Game Mode does on its own, and how
//! to undo its control.
//!
//! About is not here: the application menu has it.

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use crate::i18n::{N_, error_text, i18n, ni18n, tr};
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

/// A Unix time as the user's locale writes a date and time.
fn local_time(t: u64) -> String {
    glib::DateTime::from_unix_local(i64::try_from(t).unwrap_or(0))
        .and_then(|d| d.format("%x %X"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Who is in charge of falcond, and the one action that makes sense now:
/// hand it back while Big Game Mode manages it, take it back while it does not.
///
/// Read again whenever the row is shown, because switching Turbo on the Home
/// page takes charge too.
#[allow(clippy::too_many_lines)]
fn falcond_row() -> adw::ActionRow {
    use bigame_core::turbo::Control;

    let row = adw::ActionRow::builder()
        .title(i18n("Control of falcond"))
        .subtitle(i18n("Checking…"))
        .build();
    let release = gtk4::Button::builder()
        .label(i18n("Hand back"))
        .valign(gtk4::Align::Center)
        .visible(false)
        .build();
    let take = gtk4::Button::builder()
        .label(i18n("Take back control"))
        .valign(gtk4::Align::Center)
        .visible(false)
        .build();
    row.add_suffix(&release);
    row.add_suffix(&take);
    row.add_suffix(&info::button(
        &i18n("Control of falcond"),
        &i18n(
            "Turbo turns falcond's service on and off. The first time it did, it recorded whether falcond was enabled and running. Handing back restores exactly that and leaves falcond alone. Taking control back records falcond's state as it is then — so a later hand-back restores that — and makes it follow Turbo again: kept running and enabled if it runs, kept stopped and disabled if it does not. Switching Turbo on or off also takes control back.",
        ),
    ));

    // Show what was read. `None`: systemd could not be asked.
    let show = {
        let row = row.clone();
        let release = release.clone();
        let take = take.clone();
        move |control: Option<Control>| {
            let subtitle = match control {
                None => i18n("Could not read falcond's service"),
                Some(Control::NotInstalled) => i18n("falcond is not installed"),
                Some(Control::Managed { since }) => {
                    i18n("Managed by Big Game Mode since %s").replace("%s", &local_time(since))
                }
                Some(Control::HandedBack { at }) => {
                    i18n("Handed back on %s: falcond is as it was before Big Game Mode")
                        .replace("%s", &local_time(at))
                }
                Some(Control::NeverManaged) => {
                    i18n("Not managed: Big Game Mode has not changed falcond's service")
                }
            };
            row.set_subtitle(&subtitle);
            release.set_visible(matches!(control, Some(Control::Managed { .. })));
            release.set_sensitive(true);
            take.set_visible(control.is_some_and(Control::can_take));
            take.set_label(&if matches!(control, Some(Control::NeverManaged)) {
                i18n("Take control")
            } else {
                i18n("Take back control")
            });
            take.set_sensitive(true);
        }
    };
    let refresh = {
        let show = show.clone();
        move || {
            let show = show.clone();
            glib::spawn_future_local(async move {
                let control = gio::spawn_blocking(bigame_core::turbo::control_blocking)
                    .await
                    .ok()
                    .and_then(Result::ok);
                show(control);
            });
        }
    };
    {
        let refresh = refresh.clone();
        row.connect_map(move |_| refresh());
    }

    {
        let refresh = refresh.clone();
        release.connect_clicked(move |b| {
            let b = b.clone();
            let refresh = refresh.clone();
            glib::spawn_future_local(async move {
                b.set_sensitive(false);
                // A falcond stopped by the hand-back takes Turbo off: the
                // preset and the Booster's changes go with it, as they would
                // with Turbo off.
                let result = gio::spawn_blocking(|| {
                    let released = bigame_core::dbus_client::daemon_proxy_blocking()
                        .and_then(|p| Ok(p.release_game_backend()?))?;
                    let tidied = if released {
                        bigame_core::turbo::tidy_up_blocking()
                    } else {
                        Ok(())
                    };
                    anyhow::Ok((released, tidied))
                })
                .await;
                let text = match result {
                    Ok(Ok((true, Ok(())))) => {
                        i18n("falcond is back as it was before Big Game Mode")
                    }
                    Ok(Ok((true, Err(e)))) => format!(
                        "{}. {}: {}",
                        i18n("falcond is back as it was before Big Game Mode"),
                        i18n("Some settings could not be put back"),
                        error_text(&e)
                    ),
                    Ok(Ok((false, _))) => i18n("There was nothing to hand back"),
                    Ok(Err(e)) => {
                        crate::i18n::labelled(&i18n("Could not hand it back"), &error_text(&e))
                    }
                    Err(_) => i18n("Could not hand it back"),
                };
                crate::widgets::toast::show(&b, &text);
                refresh();
            });
        });
    }
    take.connect_clicked(move |b| {
        let b = b.clone();
        let refresh = refresh.clone();
        glib::spawn_future_local(async move {
            b.set_sensitive(false);
            let result = gio::spawn_blocking(bigame_core::turbo::take_back_blocking).await;
            let text = match result {
                Ok(Ok(_)) => i18n("Big Game Mode manages falcond again"),
                Ok(Err(e)) => {
                    crate::i18n::labelled(&i18n("Could not take control back"), &error_text(&e))
                }
                Err(_) => i18n("Could not take control back"),
            };
            crate::widgets::toast::show(&b, &text);
            refresh();
        });
    });
    row
}

/// The ping target: one of the resolvers the DNS comparison in Details
/// measures — the system's own and the public ones — or an address typed in.
///
/// A loopback address is left out: it is a local cache (systemd-resolved's
/// stub, for one), and its round trip says nothing about the network.
fn ping_rows(current: &str) -> [adw::PreferencesRow; 2] {
    let choices: Vec<(String, String)> = crate::views::details::extras::candidates()
        .into_iter()
        .filter(|r| !r.address.is_loopback())
        .map(|r| (r.address.to_string(), format!("{} · {}", r.name, r.address)))
        .collect();
    let mut labels: Vec<String> = choices.iter().map(|(_, l)| l.clone()).collect();
    labels.push(i18n("Other address"));
    let other = u32::try_from(choices.len()).unwrap_or(u32::MAX);
    let selected = choices
        .iter()
        .position(|(a, _)| a == current)
        .and_then(|i| u32::try_from(i).ok())
        .unwrap_or(other);

    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let combo = adw::ComboRow::builder()
        .title(i18n("Ping Target"))
        .subtitle(i18n("The resolvers the DNS comparison in Details measures"))
        .model(&gtk4::StringList::new(&labels))
        .selected(selected)
        .build();
    combo.add_suffix(&info::button(
        &i18n("Ping Target"),
        &i18n(
            "The address pinged for the latency readings on the Home and Details pages. The choices are the resolvers the DNS comparison measures: the ones this computer uses and well-known public ones. Any other address can be typed in.",
        ),
    ));

    let entry = adw::EntryRow::builder()
        .title(i18n("Address to ping"))
        .text(if selected == other { current } else { "" })
        .visible(selected == other)
        .build();

    {
        let entry = entry.clone();
        combo.connect_selected_notify(move |row| {
            let chosen = row.selected();
            let is_other = chosen == other;
            entry.set_visible(is_other);
            let target = if is_other {
                entry.text().trim().to_owned()
            } else {
                usize::try_from(chosen)
                    .ok()
                    .and_then(|i| choices.get(i))
                    .map(|(a, _)| a.clone())
                    .unwrap_or_default()
            };
            save_ping_target(target);
        });
    }
    entry.connect_changed(|row| save_ping_target(row.text().trim().to_owned()));
    [combo.upcast(), entry.upcast()]
}

/// Run `apply` with each state `row` is switched to. When it fails, say why
/// and put the switch back: it never shows a choice that is not kept.
fn on_switch(row: &adw::SwitchRow, apply: impl Fn(bool) -> Result<(), String> + 'static) {
    let reverting = std::rc::Rc::new(std::cell::Cell::new(false));
    row.connect_active_notify(move |row| {
        if reverting.get() {
            return;
        }
        if let Err(e) = apply(row.is_active()) {
            crate::widgets::toast::show(
                row,
                &crate::i18n::labelled(&i18n("Could not change it"), &e),
            );
            reverting.set(true);
            row.set_active(!row.is_active());
            reverting.set(false);
        }
    });
}

/// Keep `target` as the ping target, unless it is empty or would be read as
/// an option by ping.
fn save_ping_target(target: String) {
    if target.is_empty() || target.starts_with('-') {
        return;
    }
    let mut s = settings::load();
    if s.ping_target != target {
        s.ping_target = target;
        settings::save(&s);
    }
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
            "Adds a login entry for your user only (~/.config/autostart). Big Game Mode then runs in the tray and can offer a profile when a new game starts. Turning this off removes the entry.",
        ),
    );
    on_switch(&login, |on| {
        settings::set_starts_at_login(on).map_err(|e| e.to_string())
    });
    turbo.add(&login);

    turbo.add(&falcond_row());
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
    on_switch(&offer, |on| {
        let mut s = settings::load();
        s.offer_profiles = on;
        settings::try_save(&s).map_err(|e| e.to_string())
    });
    profiles.add(&offer);

    let migrate = adw::ActionRow::builder()
        .title(i18n("Profiles from an older Big Game Mode"))
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
        &i18n("Profiles from an older Big Game Mode"),
        &i18n(
            "Older versions named profiles after the game's title, which falcond can never match, and stored settings falcond ignores. Fixing renames each one to the game's real process and keeps only falcond's settings. Every profile is backed up first to ~/.local/state/bigame-mode. falcond's own profiles are never touched.",
        ),
    ));
    profiles.add(&migrate);
    // Checked when the row is first shown, not when the page is built: the
    // page is built at login with the window hidden, and the check scans
    // every launcher's library.
    let checked = std::rc::Rc::new(std::cell::Cell::new(false));
    migrate.connect_map(move |migrate| {
        if checked.replace(true) {
            return;
        }
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
                            .ok_or_else(|| {
                                bigame_core::error::UserError::plain(N_("HOME is not set"))
                            })?;
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
                            migrate.set_subtitle(&crate::i18n::labelled(
                                &i18n("Could not fix them"),
                                &error_text(&e),
                            ));
                            b.set_sensitive(true);
                        }
                        Err(_) => b.set_sensitive(true),
                    }
                });
            });
        });
    });
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
    on_switch(&notif_row, |on| {
        let mut s = settings::load();
        s.notifications_enabled = on;
        settings::try_save(&s).map_err(|e| e.to_string())
    });
    notif.add(&notif_row);
    page.add(&notif);

    // ── Monitoring ──────────────────────────────────────────────────────
    let monitoring = adw::PreferencesGroup::new();
    monitoring.set_title(&i18n("Monitoring"));
    for row in ping_rows(&current.ping_target) {
        monitoring.add(&row);
    }
    page.add(&monitoring);

    page
}
