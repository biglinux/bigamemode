//! Profiles view: the game library, with navigation to a profile editor.
//!
//! The grid shows installed games — what `bigame_core::library` found on
//! disk — and, on each, whether a profile exists. Profiles are never turned
//! into cards: falcond ships profiles for titles that may not be installed,
//! and a card for one would be a game the machine does not have. The user's
//! own profiles that match no installed game are listed apart, collapsed,
//! so they stay reachable and are never deleted by the scan.
//!
//! The scan runs off the main thread. The grid that is on screen stays until
//! the new library is ready, and is rebuilt only when something changed, so
//! coming back from the editor neither freezes the window nor flickers.
//!
//! Uses `bigame_core::profiles` for CRUD operations. Saves go through the
//! privileged helper (D-Bus, Polkit) into falcond's user profile directory.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::optimization::GameOptimization;

use crate::i18n::{error_text, i18n, ni18n};
use crate::widgets::game_card;
use crate::widgets::game_fields::GameFields;
use crate::widgets::optimization as ui;
use crate::widgets::toast;

/// Build the Profiles view with navigation stack.
#[must_use]
pub fn build() -> adw::NavigationView {
    let nav_view = adw::NavigationView::new();
    let list_page = build_list_page(&nav_view);
    nav_view.add(&list_page);
    nav_view
}

/// The library page's widgets and the last library shown in them.
struct LibraryView {
    nav: adw::NavigationView,
    /// `grid`, `empty` (no games at all) or `none` (none matches).
    stack: gtk4::Stack,
    grid: gtk4::FlowBox,
    /// The cards' entries, in the grid's order: what the filter reads.
    entries: RefCell<Vec<game_card::Entry>>,
    /// The search and the two filters.
    search: gtk4::SearchEntry,
    status_filter: adw::ToggleGroup,
    source_filter: gtk4::DropDown,
    /// The launcher names in `source_filter`, after "All launchers".
    sources: RefCell<Vec<String>>,
    /// The page shown when nothing matches.
    no_match: adw::StatusPage,
    refresh_btn: gtk4::Button,
    /// Shown when falcond has more profiles than it loads.
    limit_note: gtk4::Label,
    /// The user's profiles with no installed game, collapsed under the grid.
    others_group: adw::PreferencesGroup,
    others: adw::ExpanderRow,
    /// Rows added to `others`, removed before the next fill.
    other_rows: RefCell<Vec<adw::ActionRow>>,
    /// The library on screen; a scan whose result equals it changes nothing.
    /// The library on screen, with the games that have AI Graphics files.
    shown: RefCell<Option<(bigame_core::library::Library, HashSet<String>)>>,
    /// Whether a scan is running, and whether one was asked for meanwhile.
    scanning: RefCell<(bool, bool)>,
}

impl LibraryView {
    /// Say that `beyond` profiles are past what falcond loads, or nothing.
    fn show_limit(&self, beyond: usize) {
        self.limit_note.set_visible(beyond > 0);
        if beyond > 0 {
            self.limit_note.set_label(
                &ni18n(
                    "falcond loads at most 64 profiles, and there is %n more: one of yours is not applied.",
                    "falcond loads at most 64 profiles, and there are %n more: %n of yours are not applied.",
                    beyond,
                ),
            );
        }
    }

    /// Whether `e` passes the search and both filters: the query is found
    /// in the title or the launcher, ignoring case.
    fn matches(&self, e: &game_card::Entry) -> bool {
        let query = self.search.text().trim().to_lowercase();
        let status_ok = match self.status_filter.active_name().as_deref() {
            Some("with") => e.has_profile,
            Some("without") => !e.has_profile,
            _ => true,
        };
        let source_ok = match self.source_filter.selected() {
            0 | gtk4::INVALID_LIST_POSITION => true,
            i => self
                .sources
                .borrow()
                .get(i as usize - 1)
                .is_some_and(|s| *s == e.source),
        };
        status_ok
            && source_ok
            && (query.is_empty()
                || e.title.to_lowercase().contains(&query)
                || e.source.to_lowercase().contains(&query))
    }

    /// Filter the grid again and show the right page for the result.
    fn apply_filters(&self) {
        self.grid.invalidate_filter();
        let entries = self.entries.borrow();
        if entries.is_empty() {
            self.stack.set_visible_child_name("empty");
            return;
        }
        if entries.iter().any(|e| self.matches(e)) {
            self.stack.set_visible_child_name("grid");
            return;
        }
        let searching = !self.search.text().trim().is_empty() || self.source_filter.selected() > 0;
        let (title, text) = match (self.status_filter.active_name().as_deref(), searching) {
            (Some("without"), false) => (
                i18n("No game without a profile"),
                i18n("Every game found already has a profile."),
            ),
            (Some("with"), false) => (
                i18n("No game with a profile yet"),
                i18n("Optimize a game from its card to create one."),
            ),
            _ => (
                i18n("No games found"),
                i18n("Try another name or remove the filters."),
            ),
        };
        self.no_match.set_title(&title);
        self.no_match.set_description(Some(&text));
        self.stack.set_visible_child_name("none");
    }

    /// Back to every game.
    fn clear_filters(&self) {
        self.search.set_text("");
        self.status_filter.set_active_name(Some("all"));
        self.source_filter.set_selected(0);
        self.apply_filters();
        self.search.grab_focus();
    }

    /// Offer exactly the launchers the library has.
    fn set_sources(&self, entries: &[game_card::Entry]) {
        let mut sources: Vec<String> = entries.iter().map(|e| e.source.clone()).collect();
        sources.sort();
        sources.dedup();
        if *self.sources.borrow() == sources {
            return;
        }
        let keep = match self.source_filter.selected() {
            0 | gtk4::INVALID_LIST_POSITION => None,
            i => self.sources.borrow().get(i as usize - 1).cloned(),
        };
        let mut labels = vec![i18n("All launchers")];
        labels.extend(sources.iter().cloned());
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.source_filter
            .set_model(Some(&gtk4::StringList::new(&refs)));
        let selected = keep
            .and_then(|k| sources.iter().position(|s| *s == k))
            .and_then(|i| u32::try_from(i + 1).ok())
            .unwrap_or(0);
        *self.sources.borrow_mut() = sources;
        self.source_filter.set_selected(selected);
        // One launcher: nothing to choose.
        self.source_filter
            .set_visible(self.sources.borrow().len() > 1);
    }
}

/// Build the profile list page.
#[allow(clippy::too_many_lines)]
fn build_list_page(nav_view: &adw::NavigationView) -> adw::NavigationPage {
    let page = adw::PreferencesPage::new();

    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Game Library"));
    group.set_description(Some(&i18n(
        "Games installed on this computer, and the profiles that tune them.",
    )));

    // A poster grid rather than a list: a library reads far faster as cover
    // art than as rows of text, especially when most entries are titles the
    // user recognises by their box art. Cards are a fixed size (see
    // `game_card`), so the grid only ever changes its number of columns; the
    // cells share the row's spare width, and each card sits centred in its
    // cell.
    let grid = gtk4::FlowBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .homogeneous(true)
        .column_spacing(12)
        .row_spacing(18)
        .min_children_per_line(2)
        .max_children_per_line(8)
        .valign(gtk4::Align::Start)
        .build();

    grid.add_css_class("game-library");

    let empty = adw::StatusPage::builder()
        .icon_name("applications-games-symbolic")
        .title(i18n("No games found"))
        .description(i18n(
            "Install a game through Steam, Lutris, Heroic or the application menu, then rescan.",
        ))
        .build();
    empty.add_css_class("compact");

    // Nothing matches the search and filters: say so, with the way back.
    let clear_btn = gtk4::Button::builder()
        .label(i18n("Clear filters"))
        .halign(gtk4::Align::Center)
        .css_classes(["pill"])
        .build();
    let no_match = adw::StatusPage::builder()
        .icon_name("system-search-symbolic")
        .title(i18n("No games found"))
        .description(i18n("Try another name or remove the filters."))
        .child(&clear_btn)
        .build();
    no_match.add_css_class("compact");

    let stack = gtk4::Stack::new();
    stack.add_named(&grid, Some("grid"));
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&no_match, Some("none"));
    stack.set_vhomogeneous(false);
    stack.set_hhomogeneous(false);

    // Search and filters: instant, over what is on screen — typing filters
    // the cards already built, it never rescans the library.
    let search = gtk4::SearchEntry::builder()
        .placeholder_text(i18n("Search games…"))
        .hexpand(true)
        .build();
    search.update_property(&[gtk4::accessible::Property::Label(&i18n("Search games"))]);
    let source_filter = gtk4::DropDown::from_strings(&[&i18n("All launchers")]);
    source_filter.set_valign(gtk4::Align::Center);
    source_filter.set_tooltip_text(Some(&i18n("Launcher")));
    source_filter.update_property(&[gtk4::accessible::Property::Label(&i18n("Launcher"))]);
    let status_filter = adw::ToggleGroup::new();
    for (name, label) in [
        ("all", i18n("All")),
        ("with", i18n("With a profile")),
        ("without", i18n("Without a profile")),
    ] {
        let t = adw::Toggle::new();
        t.set_name(Some(name));
        t.set_label(Some(&label));
        status_filter.add(t);
    }
    status_filter.set_active_name(Some("all"));
    status_filter.set_halign(gtk4::Align::Start);
    let top = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    top.append(&search);
    top.append(&source_filter);
    let filters = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    filters.add_css_class("library-filters");
    filters.set_margin_bottom(12);
    filters.append(&top);
    filters.append(&status_filter);

    // Creating a profile: the wizard is the way to recommend — every option
    // explained, one step at a time — so it is the main button, with a
    // second line that says so; a blank profile is the alternative beside it.
    let wizard_btn = wizard_button();
    let add_btn = gtk4::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("list-add-symbolic")
                .label(i18n("New Profile"))
                .build(),
        )
        .tooltip_text(i18n("A blank profile, every option set by hand"))
        .css_classes(["flat", "pill"])
        .valign(gtk4::Align::Center)
        .build();
    let create = adw::WrapBox::builder()
        .child_spacing(8)
        .line_spacing(8)
        .margin_bottom(12)
        .build();
    create.append(&wizard_btn);
    create.append(&add_btn);
    let limit_note = gtk4::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .margin_bottom(12)
        .css_classes(["warning"])
        .build();
    group.add(&limit_note);
    group.add(&create);
    group.add(&filters);
    group.add(&stack);

    let import_btn = gtk4::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(i18n("Import Profile"))
        .css_classes(["circular", "flat"])
        .build();
    import_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Import Profile"))]);
    let refresh_btn = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(i18n("Rescan game library"))
        .css_classes(["circular", "flat"])
        .build();
    refresh_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Rescan game library",
    ))]);
    let hdr = gtk4::Box::builder().spacing(4).build();
    hdr.append(&refresh_btn);
    hdr.append(&import_btn);
    group.set_header_suffix(Some(&hdr));
    page.add(&group);

    // Profiles of the user's that match no installed game: written by hand
    // for a game no launcher lists, imported, or left from a game since
    // removed. They are not games, so they are not cards, but they are the
    // user's and stay editable here. Hidden when there are none.
    let others_group = adw::PreferencesGroup::new();
    others_group.set_visible(false);
    let others = adw::ExpanderRow::builder()
        .subtitle(i18n(
            "Profiles of yours that match no installed game. They stay until you delete them.",
        ))
        .build();
    others_group.add(&others);
    page.add(&others_group);

    let view = Rc::new(LibraryView {
        nav: nav_view.clone(),
        stack,
        grid,
        entries: RefCell::new(Vec::new()),
        search: search.clone(),
        status_filter: status_filter.clone(),
        source_filter: source_filter.clone(),
        sources: RefCell::new(Vec::new()),
        no_match,
        refresh_btn: refresh_btn.clone(),
        limit_note,
        others_group,
        others,
        other_rows: RefCell::new(Vec::new()),
        shown: RefCell::new(None),
        scanning: RefCell::new((false, false)),
    });

    // The filter reads the entries by the card's position in the grid.
    {
        let weak = Rc::downgrade(&view);
        view.grid.set_filter_func(move |child| {
            weak.upgrade().is_none_or(|v| {
                usize::try_from(child.index())
                    .ok()
                    .and_then(|i| v.entries.borrow().get(i).map(|e| v.matches(e)))
                    .unwrap_or(true)
            })
        });
    }
    {
        let v = Rc::clone(&view);
        search.connect_search_changed(move |_| v.apply_filters());
    }
    {
        let v = Rc::clone(&view);
        search.connect_stop_search(move |s| {
            s.set_text("");
            v.apply_filters();
        });
    }
    // Typing anywhere on the page goes to the search.
    search.set_key_capture_widget(Some(&page));
    {
        let v = Rc::clone(&view);
        status_filter.connect_active_name_notify(move |_| v.apply_filters());
    }
    {
        let v = Rc::clone(&view);
        source_filter.connect_selected_notify(move |_| v.apply_filters());
    }
    {
        let v = Rc::clone(&view);
        clear_btn.connect_clicked(move |_| v.clear_filters());
    }

    // Rescanned whenever the list comes on screen: the first time the page is
    // shown, on return from a detail or create page, and after a profile was
    // made elsewhere (from Home or the notification offer). The grid already
    // on screen stays until the scan is done.
    {
        let on_map = Rc::clone(&view);
        view.grid.connect_map(move |_| refresh_library(&on_map));
    }

    // Refreshing is driven by navigation and by the explicit button above,
    // not by a timer: with cover art, a poll would re-read the whole library
    // in a window the user may not even be looking at.
    {
        let view = Rc::clone(&view);
        refresh_btn.connect_clicked(move |_| refresh_library(&view));
    }

    // "Create with Wizard" → the guided profile, then a rescan.
    {
        let view = Rc::clone(&view);
        wizard_btn.connect_clicked(move |btn| {
            let view = Rc::clone(&view);
            let anchor = btn.clone();
            crate::views::profile_wizard::open(btn, move |_| {
                toast::show(&anchor, &i18n("Profile created"));
                refresh_library(&view);
            });
        });
    }

    // "New Profile" → empty detail page
    {
        let nav = nav_view.clone();
        add_btn.connect_clicked(move |_| {
            nav.push(&build_new_page("", None));
        });
    }

    {
        let view = Rc::clone(&view);
        import_btn.connect_clicked(move |btn| {
            let dialog = gtk4::FileDialog::builder()
                .title(i18n("Import Profile"))
                .build();
            let filter = gtk4::FileFilter::new();
            filter.add_pattern("*.conf");
            filter.add_pattern("*.toml");
            filter.set_name(Some(&format!("{} (*.conf, *.toml)", i18n("Profile files"))));
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let btn_ref = btn.clone();
            let view = Rc::clone(&view);
            let win = btn.root().and_downcast::<gtk4::Window>();
            dialog.open(win.as_ref(), gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        gtk4::glib::spawn_future_local(async move {
                            // Saving goes through the helper and may wait on a
                            // Polkit prompt: off the main thread.
                            let result =
                                gio::spawn_blocking(move || bigame_core::profiles::import(&path))
                                    .await
                                    .unwrap_or_else(|_| Err(anyhow::anyhow!("import panicked")));
                            match result {
                                Ok(name) => {
                                    toast::show(&btn_ref, &i18n("Profile imported"));
                                    refresh_library(&view);
                                    view.nav.push(&build_detail_page(&name, None));
                                }
                                Err(e) => {
                                    toast::show(
                                        &btn_ref,
                                        &i18n("Import failed: %s").replace("%s", &error_text(&e)),
                                    );
                                }
                            }
                        });
                    }
                }
            });
        });
    }

    // Drag-and-drop import
    {
        let view = Rc::clone(&view);
        let drop_target =
            gtk4::DropTarget::new(gio::File::static_type(), gtk4::gdk::DragAction::COPY);
        drop_target.connect_drop(move |target, value, _x, _y| {
            let Some(file) = value.get::<gio::File>().ok() else {
                return false;
            };
            let Some(path) = file.path() else {
                return false;
            };
            let view = Rc::clone(&view);
            let target_ref = target.clone();
            gtk4::glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || bigame_core::profiles::import(&path))
                    .await
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("import panicked")));
                let Some(widget) = target_ref.widget() else {
                    return;
                };
                match result {
                    Ok(name) => {
                        refresh_library(&view);
                        toast::show(&widget, &i18n("Profile imported via drag-and-drop"));
                        view.nav.push(&build_detail_page(&name, None));
                    }
                    Err(e) => toast::show(
                        &widget,
                        &i18n("Import failed: %s").replace("%s", &error_text(&e)),
                    ),
                }
            });
            true
        });
        page.add_controller(drop_target);
    }

    adw::NavigationPage::builder()
        .title(i18n("Game Library"))
        .child(&page)
        .build()
}

/// The wizard's button: the recommended way to make a profile, said on it.
fn wizard_button() -> gtk4::Button {
    let icon = gtk4::Image::from_icon_name("bigame-wizard-symbolic");
    icon.set_pixel_size(20);
    icon.set_accessible_role(gtk4::AccessibleRole::Presentation);
    let title = gtk4::Label::builder()
        .label(i18n("Create with Wizard"))
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    let subtitle = gtk4::Label::builder()
        .label(i18n("Recommended · every option explained"))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["caption"])
        .build();
    let lines = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    lines.set_valign(gtk4::Align::Center);
    lines.append(&title);
    lines.append(&subtitle);
    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    content.append(&icon);
    content.append(&lines);
    let button = gtk4::Button::builder()
        .child(&content)
        .tooltip_text(i18n(
            "A guided profile: every option explained, for a game you pick",
        ))
        .css_classes(["suggested-action", "wizard-button"])
        .build();
    button.update_property(&[
        gtk4::accessible::Property::Label(&i18n("Create with Wizard")),
        gtk4::accessible::Property::Description(&i18n("Recommended · every option explained")),
    ]);
    button
}

/// The editor for the profile in file `stem`, each part read from its owner
/// (`GameOptimization::load`). `card` is the game's card, when the editor
/// was opened from one: its title and install folder.
fn build_detail_page(stem: &str, card: Option<&game_card::Entry>) -> adw::NavigationPage {
    build_editor(GameOptimization::load(stem), card, false)
}

/// The editor for a new profile keyed on `process`.
fn build_new_page(process: &str, card: Option<&game_card::Entry>) -> adw::NavigationPage {
    build_editor(GameOptimization::new(process), card, true)
}

/// A game's profile: the same sections as Tuning, for this game only.
///
/// What the editor needs to know about the machine and the game (sched-ext
/// over the system bus, `/usr/bin`, `OptiScaler` in the game's folder) is read
/// off the main thread; the page shows a spinner until then.
fn build_editor(
    g: GameOptimization,
    card: Option<&game_card::Entry>,
    is_new: bool,
) -> adw::NavigationPage {
    let title = card.map_or_else(|| g.profile.name.clone(), |c| c.title.clone());
    let spinner = adw::Spinner::builder()
        .width_request(32)
        .height_request(32)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .build();
    let nav_page = adw::NavigationPage::builder()
        .title(if is_new {
            i18n("New Profile")
        } else if title.is_empty() {
            g.profile.name.clone()
        } else {
            title.clone()
        })
        .child(&spinner)
        .build();
    let card = card.cloned();
    let target = card.as_ref().and_then(|c| c.target.clone());
    let process = g.profile.name.clone();
    let shown = nav_page.clone();
    glib::spawn_future_local(async move {
        let read = gio::spawn_blocking(move || {
            (
                ui::Machine::detect(),
                ui::Game::detect(&process, &title, target),
            )
        })
        .await;
        match read {
            Ok((m, game)) => shown.set_child(Some(&editor(g, card.as_ref(), is_new, &m, game))),
            Err(_) => shown.set_child(Some(
                &adw::StatusPage::builder()
                    .icon_name("dialog-error-symbolic")
                    .title(i18n("the worker thread stopped"))
                    .build(),
            )),
        }
    });
    nav_page
}

/// [`build_editor`]'s page once the machine `m` and the `game` are read.
///
/// The rows are `widgets::game_fields`', the same the wizard shows one per
/// step, over the same `GameOptimization`; Save writes each part to its
/// owner and says which part did not take.
#[allow(clippy::too_many_lines)]
fn editor(
    g: GameOptimization,
    card: Option<&game_card::Entry>,
    is_new: bool,
    m: &ui::Machine,
    mut game: ui::Game,
) -> adw::ToolbarView {
    let page = adw::PreferencesPage::new();
    // What reaches the game: its Steam launch options, Big Game Mode's own
    // launch, its settings in Heroic, or — started through another
    // launcher (Lutris, Flatpak) — nothing of the game's own.
    let steam = source_label(bigame_core::games::Source::Steam);
    game.reach = match card {
        Some(c) if c.source == steam => ui::Reach::Steam,
        Some(c) if matches!(c.launch, Some(game_card::Launch::Direct(..))) => ui::Reach::Launch,
        Some(game_card::Entry {
            heroic: Some(flatpak),
            ..
        }) => ui::Reach::Heroic { flatpak: *flatpak },
        Some(_) => ui::Reach::Nothing,
        None => ui::Reach::Unknown,
    };

    page.add(&ui::scope_banner(ui::Scope::Game(&game.title)));

    // The process name: first for a new profile, which needs one; in
    // Advanced for one that exists, where it is rarely changed and should
    // not take the focus (typing would replace it).
    let name_row = adw::EntryRow::builder()
        .title(i18n("Process Name"))
        .text(&g.profile.name)
        .build();
    name_row.add_prefix(&gtk4::Image::from_icon_name(
        "application-x-executable-symbolic",
    ));
    name_row.add_suffix(&crate::widgets::info::button(
        &i18n("Process Name"),
        &i18n("falcond recognises the game by the name of its process (as the system monitor shows it), not by its title. A profile under another name never applies."),
    ));
    if is_new {
        let identity = adw::PreferencesGroup::new();
        identity.add(&name_row);
        page.add(&identity);
    }

    let fields = Rc::new(GameFields::build(&g, m, &game, true));
    page.add(&fields.performance.group);
    page.add(&fields.gamescope.group);
    page.add(&fields.image_quality.group);
    page.add(&fields.frame_generation.group);
    page.add(&fields.mangohud.group);
    page.add(&advanced_group(
        &g.profile.name,
        (!is_new).then_some(&name_row),
        game.reach,
    ));

    // Save sits in a bar that stays on screen, not at the end of the page.
    let save_btn = gtk4::Button::builder()
        .label(i18n("Save Profile"))
        .css_classes(["suggested-action", "pill"])
        .build();
    let save_hint = gtk4::Label::builder()
        .label(i18n("Changes apply when you save"))
        .css_classes(["dim-label", "caption"])
        .wrap(true)
        .xalign(0.0)
        .hexpand(true)
        .build();
    let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    bar.add_css_class("editor-save-bar");
    bar.set_margin_top(8);
    bar.set_margin_bottom(8);
    bar.set_margin_start(16);
    bar.set_margin_end(16);
    bar.append(&save_hint);
    bar.append(&save_btn);

    let shared = Rc::new(RefCell::new(g));
    // The profile is on disk: it was opened, or has been saved since.
    let exists = Rc::new(std::cell::Cell::new(!is_new));
    // What the page shows now, as a profile, or why it cannot be saved.
    let edited: Rc<dyn Fn() -> Result<GameOptimization, String>> = {
        let (shared, fields) = (Rc::clone(&shared), Rc::clone(&fields));
        Rc::new(move || {
            let mut g = shared.borrow().clone();
            name_row.text().trim().clone_into(&mut g.profile.name);
            fields.apply(&mut g);
            let (errors, _) = g.problems();
            if errors.is_empty() {
                Ok(g)
            } else {
                Err(errors
                    .iter()
                    .map(|e| i18n(e))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        })
    };
    {
        let (shared, edited, exists) = (Rc::clone(&shared), Rc::clone(&edited), Rc::clone(&exists));
        save_btn.connect_clicked(move |btn| {
            let g = match edited() {
                Ok(g) => g,
                Err(errors) => {
                    crate::widgets::toast::error(btn, &i18n("The profile was not saved"), &errors);
                    return;
                }
            };
            let (_, warnings) = g.problems();
            if !warnings.is_empty() {
                let soft: Vec<String> = warnings.iter().map(|w| i18n(w)).collect();
                toast::show(btn, &format!("⚠ {}", soft.join("; ")));
            }
            btn.set_sensitive(false);
            btn.set_label(&i18n("Saving…"));
            let (b, exists) = (btn.clone(), Rc::clone(&exists));
            save_profile(btn.upcast_ref(), &shared, g, move |saved| {
                if saved {
                    exists.set(true);
                }
                b.set_sensitive(true);
                b.set_label(&i18n("Save Profile"));
            });
        });
    }

    let toolbar = adw::ToolbarView::new();
    let detail_header = adw::HeaderBar::new();
    detail_header.set_show_end_title_buttons(false);
    detail_header.set_show_start_title_buttons(false);
    // Start the game from here, with Turbo and this profile: the same launch
    // as the card's "Launch (Turbo)".
    if let Some(entry) = card.filter(|c| c.launch.is_some()) {
        detail_header.pack_end(&launch_button(
            entry.clone(),
            &shared,
            &edited,
            &exists,
            &save_btn,
        ));
    }
    if !is_new {
        add_header_actions(&detail_header, &shared.borrow().profile.name);
    }
    toolbar.add_top_bar(&detail_header);
    toolbar.set_content(Some(&page));
    toolbar.add_bottom_bar(&bar);
    toolbar
}

/// Save `g` off the main thread — the helper may wait on a Polkit password
/// prompt, and the window must keep drawing meanwhile — keep it as the
/// page's profile, and say what took. `done` is told whether it was saved.
fn save_profile(
    anchor: &gtk4::Widget,
    shared: &Rc<RefCell<GameOptimization>>,
    g: GameOptimization,
    done: impl FnOnce(bool) + 'static,
) {
    let (anchor, shared) = (anchor.clone(), Rc::clone(shared));
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || {
            let mut g = g;
            g.save().map(|report| (g, report))
        })
        .await;
        let saved = match result {
            Ok(Ok((saved, report))) => {
                *shared.borrow_mut() = saved;
                crate::widgets::optimization::report_save(&anchor, &report);
                true
            }
            Ok(Err(e)) => {
                crate::widgets::toast::error(
                    &anchor,
                    &i18n("The profile was not saved"),
                    &error_text(&e),
                );
                false
            }
            Err(_) => {
                crate::widgets::toast::error(
                    &anchor,
                    &i18n("The profile was not saved"),
                    &i18n("the worker thread stopped"),
                );
                false
            }
        };
        done(saved);
    });
}

/// "Launch (Turbo)" at the head of a game's profile.
///
/// It checks the profile, offers to save what is not saved yet (a profile
/// that is not saved does not apply), switches Turbo on through Home's
/// `app.turbo` when it is off, and starts the game as its card does.
fn launch_button(
    entry: game_card::Entry,
    shared: &Rc<RefCell<GameOptimization>>,
    edited: &Rc<dyn Fn() -> Result<GameOptimization, String>>,
    exists: &Rc<std::cell::Cell<bool>>,
    save_btn: &gtk4::Button,
) -> gtk4::Button {
    let button = gtk4::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("media-playback-start-symbolic")
                .label(i18n("Launch (Turbo)"))
                .build(),
        )
        .tooltip_text(i18n("Start this game with Turbo on and this profile"))
        .css_classes(["suggested-action"])
        .build();
    let (shared, edited, exists, save_btn) = (
        Rc::clone(shared),
        Rc::clone(edited),
        Rc::clone(exists),
        save_btn.clone(),
    );
    button.connect_clicked(move |btn| {
        let g = match edited() {
            Ok(g) => g,
            Err(errors) => {
                crate::widgets::toast::error(
                    btn,
                    &i18n("The game was not started: the profile has errors"),
                    &errors,
                );
                return;
            }
        };
        let anchor: gtk4::Widget = btn.clone().upcast();
        let entry = entry.clone();
        let is_saved = exists.get() && g == *shared.borrow();
        if is_saved {
            with_turbo(&anchor, entry);
            return;
        }
        // Unsaved changes: saved first, or left behind on purpose.
        let dialog = adw::AlertDialog::builder()
            .heading(i18n("Save the profile first?"))
            .body(i18n(
                "The game gets this profile only once it is saved. Without saving, it starts with the profile as it was.",
            ))
            .build();
        dialog.add_response("cancel", &i18n("Cancel"));
        dialog.add_response("launch", &i18n("Launch without saving"));
        dialog.add_response("save", &i18n("Save and launch"));
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        let (shared, exists, save_btn) = (Rc::clone(&shared), Rc::clone(&exists), save_btn.clone());
        dialog.connect_response(None, move |_, response| match response {
            "launch" => with_turbo(&anchor, entry.clone()),
            "save" => {
                save_btn.set_sensitive(false);
                let (anchor, entry, save_btn, exists) =
                    (anchor.clone(), entry.clone(), save_btn.clone(), Rc::clone(&exists));
                save_profile(&anchor.clone(), &shared, g.clone(), move |saved| {
                    save_btn.set_sensitive(true);
                    if saved {
                        exists.set(true);
                        with_turbo(&anchor, entry);
                    }
                });
            }
            _ => {}
        });
        dialog.present(Some(btn));
    });
    button
}

/// Switch Turbo on when it is off — through Home's `app.turbo`, the one
/// switch — and start `entry`'s game once it has settled. A Turbo that
/// could not be switched on is said, and the game starts all the same.
fn with_turbo(anchor: &gtk4::Widget, entry: game_card::Entry) {
    let Some(app) = gio::Application::default() else {
        launch_game(&entry, anchor);
        return;
    };
    let turbo_on = || {
        app.action_state("turbo")
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    };
    if turbo_on() {
        launch_game(&entry, anchor);
        return;
    }
    // Settled: the action takes requests again. Then on or not, the game
    // starts, once.
    let pending = Rc::new(RefCell::new(Some((entry, anchor.clone()))));
    let handlers: Rc<RefCell<Vec<glib::SignalHandlerId>>> = Rc::new(RefCell::new(Vec::new()));
    let settle = {
        let (pending, handlers) = (Rc::clone(&pending), Rc::clone(&handlers));
        Rc::new(move |app: &gio::Application| {
            if !app.is_action_enabled("turbo") {
                return;
            }
            let Some((entry, anchor)) = pending.borrow_mut().take() else {
                return;
            };
            for id in handlers.borrow_mut().drain(..) {
                app.disconnect(id);
            }
            let on = app
                .action_state("turbo")
                .and_then(|v| v.get::<bool>())
                .unwrap_or(false);
            if !on {
                toast::show(
                    &anchor,
                    &i18n(
                        "Turbo could not be switched on; the game starts without its profile. See Home.",
                    ),
                );
            }
            launch_game(&entry, &anchor);
        })
    };
    {
        let settle = Rc::clone(&settle);
        handlers
            .borrow_mut()
            .push(
                app.connect_action_enabled_changed(Some("turbo"), move |group, _, _| {
                    if let Some(app) = group.downcast_ref::<gio::Application>() {
                        settle(app);
                    }
                }),
            );
    }
    toast::show(anchor, &i18n("Switching Turbo on…"));
    app.change_action_state("turbo", &true.to_variant());
    // Asked while Turbo was already switching: it settles on its own.
    if !app.is_action_enabled("turbo") {
        return;
    }
    // The request did not start a switch (it was refused): start anyway.
    glib::idle_add_local_once(move || {
        if let Some(app) = gio::Application::default() {
            if app.is_action_enabled("turbo") && pending.borrow().is_some() {
                settle(&app);
            }
        }
    });
}

/// Advanced, last as in Tuning: the process name (for a profile that
/// exists) and where this game's settings are kept, each with its ⓘ.
fn advanced_group(
    process: &str,
    name_row: Option<&adw::EntryRow>,
    reach: ui::Reach,
) -> adw::PreferencesGroup {
    use crate::widgets::launch::icon;
    let group = ui::section(&i18n("Advanced"));
    group.set_description(Some(&i18n(
        "Where this game's settings are kept. Nothing here needs changing to play.",
    )));
    if let Some(row) = name_row {
        group.add(row);
    }
    let file = |title: &str, path: String, icon_name: &str, about: &str| {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(path)
            .subtitle_lines(2)
            .use_markup(false)
            .build();
        row.set_subtitle_selectable(true);
        icon(&row, icon_name);
        row.add_suffix(&crate::widgets::info::button(title, about));
        row
    };
    let stem = if process.is_empty() { "…" } else { process };
    group.add(&file(
        &i18n("Profile file"),
        format!("{}/{stem}.conf", bigame_core::profiles::USER_PROFILES_DIR),
        "text-x-generic-symbolic",
        &i18n(
            "falcond's profile for this game: performance mode, the scheduler, 3D V-Cache and whether Gamescope wraps it. It is root's file, so Big Game Mode writes it through its privileged helper, which may ask for your password, and falcond reloads it.",
        ),
    ));
    group.add(&file(
        &i18n("This game's settings"),
        bigame_core::game_settings::dir()
            .join(format!("{stem}.toml"))
            .to_string_lossy()
            .into_owned(),
        "folder-symbolic",
        &i18n(
            "Big Game Mode's own choices for this game — its launch settings, MangoHud and AI Graphics — kept in your configuration, with no password. What Big Game Mode wrote into Steam's launch options or Heroic's settings is recorded there, so it changes or removes exactly that.",
        ),
    ));
    if reach == ui::Reach::Steam && !process.is_empty() {
        let row = file(
            &i18n("Steam launch options"),
            i18n("Reading…"),
            "utilities-terminal-symbolic",
            &i18n(
                "What Steam gives this game when it starts it. When you save, Big Game Mode puts its Gamescope wrapper and its variables here (with Steam closed) and takes out only what it put there; the rest is yours.",
            ),
        );
        group.add(&row);
        let process = process.to_owned();
        glib::spawn_future_local(async move {
            let options = gio::spawn_blocking(move || {
                bigame_core::steam_gamescope::current_options(&process)
            })
            .await
            .ok()
            .flatten();
            row.set_subtitle(&options.unwrap_or_else(|| i18n("none")));
        });
    }
    if matches!(reach, ui::Reach::Heroic { .. }) && !process.is_empty() {
        let row = file(
            &i18n("Heroic's settings for this game"),
            i18n("Reading…"),
            "text-x-generic-symbolic",
            &i18n(
                "The file Heroic keeps this game's settings in. When you save, Big Game Mode writes the game's own Gamescope, Wine FSR and vkBasalt there (with Heroic closed, after keeping a copy of your file) and later takes out only what it wrote; the rest is yours.",
            ),
        );
        group.add(&row);
        let process = process.to_owned();
        glib::spawn_future_local(async move {
            let files = gio::spawn_blocking(move || {
                bigame_core::heroic_launch::targets(&process)
                    .iter()
                    .map(|t| t.file().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .await
            .unwrap_or_default();
            row.set_subtitle(&if files.is_empty() {
                i18n("none")
            } else {
                files
            });
        });
    }
    group
}

/// Export and Delete, for a profile that exists.
fn add_header_actions(header: &adw::HeaderBar, name: &str) {
    let export_btn = gtk4::Button::builder()
        .icon_name("document-save-as-symbolic")
        .tooltip_text(i18n("Export Profile"))
        .build();
    export_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Export Profile"))]);
    let export_name = name.to_owned();
    export_btn.connect_clicked(move |btn| {
        let dialog = gtk4::FileDialog::builder()
            .title(i18n("Export Profile"))
            .initial_name(format!("{export_name}.conf"))
            .build();
        let btn_ref = btn.clone();
        let name = export_name.clone();
        let win = btn.root().and_downcast::<gtk4::Window>();
        dialog.save(win.as_ref(), gio::Cancellable::NONE, move |result| {
            if let Some(path) = result.ok().and_then(|f| f.path()) {
                match bigame_core::profiles::export(&name, &path) {
                    Ok(()) => toast::show(&btn_ref, &i18n("Profile exported")),
                    Err(e) => toast::show(
                        &btn_ref,
                        &i18n("Export failed: %s").replace("%s", &error_text(&e)),
                    ),
                }
            }
        });
    });
    header.pack_end(&export_btn);

    // No "activate" button: falcond applies a game's profile by itself
    // when the game's process starts.
    let delete_btn = gtk4::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text(i18n("Delete Profile"))
        .css_classes(["destructive-action"])
        .build();
    delete_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Delete Profile"))]);
    let profile_name = name.to_owned();
    delete_btn.connect_clicked(move |btn| {
        let dialog = adw::AlertDialog::builder()
            .heading(i18n("Delete Profile?"))
            .body(i18n("Remove \"%s\" permanently?").replace("%s", &profile_name))
            .build();
        dialog.add_response("cancel", &i18n("Cancel"));
        dialog.add_response("delete", &i18n("Delete"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let name = profile_name.clone();
        let btn_ref = btn.clone();
        dialog.connect_response(None, move |_dlg, response| {
            if response != "delete" {
                return;
            }
            // Report what happened, not what was attempted.
            let n = name.clone();
            btn_ref.set_sensitive(false);
            let feedback = btn_ref.clone();
            glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || bigame_core::profiles::delete(&n)).await;
                match result {
                    Ok(Ok(())) => toast::show(&feedback, &i18n("Profile deleted")),
                    Ok(Err(e)) => {
                        feedback.set_sensitive(true);
                        toast::show(
                            &feedback,
                            &i18n("Could not delete profile: %s").replace("%s", &error_text(&e)),
                        );
                    }
                    Err(_) => {
                        feedback.set_sensitive(true);
                        toast::show(&feedback, &i18n("Could not delete profile"));
                    }
                }
            });
        });
        dialog.present(Some(btn));
    });
    header.pack_end(&delete_btn);
}

/// Scan the machine off the main thread and show the result.
///
/// One scan at a time: a request made while one runs is honoured once it
/// finishes, so a burst of requests (import, then map) costs two scans, not
/// a pile-up. What is on screen is replaced only when the library changed.
fn refresh_library(view: &Rc<LibraryView>) {
    {
        let mut scanning = view.scanning.borrow_mut();
        if scanning.0 {
            scanning.1 = true;
            return;
        }
        scanning.0 = true;
    }
    // A discreet sign of work in the button that asks for it.
    view.refresh_btn.set_child(Some(&adw::Spinner::new()));
    view.refresh_btn.set_sensitive(false);

    let view = Rc::clone(view);
    glib::spawn_future_local(async move {
        let (scanned, beyond) = gio::spawn_blocking(|| {
            (
                (
                    bigame_core::library::scan(),
                    bigame_core::graphics::installed_processes(&bigame_core::graphics::state_dir()),
                ),
                bigame_core::profiles::beyond_falcond_limit(&bigame_core::profiles::index()),
            )
        })
        .await
        .unwrap_or_default();
        view.show_limit(beyond);
        if view.shown.borrow().as_ref() != Some(&scanned) {
            show_library(&view, &scanned.0, &scanned.1);
            *view.shown.borrow_mut() = Some(scanned);
        }
        view.refresh_btn.set_icon_name("view-refresh-symbolic");
        view.refresh_btn.set_sensitive(true);
        let again = {
            let mut scanning = view.scanning.borrow_mut();
            scanning.0 = false;
            std::mem::take(&mut scanning.1)
        };
        if again {
            refresh_library(&view);
        }
    });
}

/// Put `library` on screen, in one pass, so the change is one frame.
fn show_library(
    view: &Rc<LibraryView>,
    library: &bigame_core::library::Library,
    ai_installed: &HashSet<String>,
) {
    while let Some(child) = view.grid.first_child() {
        view.grid.remove(&child);
    }
    let entries: Vec<game_card::Entry> = library
        .games
        .iter()
        .map(|e| card_entry(e, ai_installed))
        .collect();
    view.entries.borrow_mut().clone_from(&entries);
    view.set_sources(&entries);
    for entry in entries {
        let nav_activate = view.nav.clone();
        let nav_menu = view.nav.clone();
        // Weak: the cards belong to the view, and the view must not be kept
        // alive by its own cards.
        let library = Rc::downgrade(view);
        let card = game_card::build(
            &entry,
            move |entry| open_profile(entry, &nav_activate),
            move |entry, anchor| show_card_menu(entry, anchor, &nav_menu, &library),
        );
        view.grid.insert(&card, -1);
        // The card is the focus stop, not the cell around it: one ring,
        // one Tab per game.
        if let Some(cell) = view.grid.last_child() {
            cell.set_focusable(false);
        }
    }
    view.apply_filters();

    for row in view.other_rows.borrow_mut().drain(..) {
        view.others.remove(&row);
    }
    let others = &library.unmatched_profiles;
    view.others_group.set_visible(!others.is_empty());
    view.others.set_title(&ni18n(
        "%n profile without an installed game",
        "%n profiles without an installed game",
        others.len(),
    ));
    for profile in others {
        let row = adw::ActionRow::builder()
            .title(&profile.name)
            .subtitle(i18n("Custom profile"))
            .activatable(true)
            .build();
        row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
        let nav = view.nav.clone();
        let stem = profile.stem.clone();
        row.connect_activated(move |_| nav.push(&build_detail_page(&stem, None)));
        view.others.add_row(&row);
        view.other_rows.borrow_mut().push(row);
    }
}

/// A library entry as its card shows it.
fn card_entry(
    entry: &bigame_core::library::Entry,
    ai_installed: &HashSet<String>,
) -> game_card::Entry {
    let game = &entry.game;
    let key = entry.key().to_owned();
    game_card::Entry {
        title: game.name.clone(),
        source: source_label(game.source),
        cover: game.cover.clone(),
        icon: game.icon.clone(),
        has_profile: entry.profile.is_some(),
        system_profile: entry.profile.as_ref().is_some_and(|p| p.system),
        profile_stem: entry.profile.as_ref().map(|p| p.stem.clone()),
        launch_command: game.launch_command.clone(),
        key_is_verified: game.has_real_executable(),
        target: game
            .install_path
            .clone()
            .map(|root| bigame_core::graphics::Target {
                name: game.name.clone(),
                process: key.clone(),
                app_id: game.app_id.clone(),
                install_root: root,
            }),
        launch: launch_command(game),
        heroic: match &game.launcher {
            Some(r @ bigame_core::games::LauncherRef::Heroic { .. }) => {
                Some(r.flatpak_id().is_some())
            }
            _ => None,
        },
        ai_installed: ai_installed.contains(&key.to_lowercase()),
        key,
    }
}

/// How a game is started from here: one with a native executable by
/// Big Game Mode itself, with its launch settings; a Steam title, and any other
/// game its launcher records, by that launcher (`steam -applaunch <id>`,
/// Heroic's `heroic://launch` link, `lutris:rungame/<slug>`, `flatpak run
/// <id>`). `None` when there is neither, rather than guessing a program name
/// and running whatever the PATH resolves it to.
fn launch_command(game: &bigame_core::games::DetectedGame) -> Option<game_card::Launch> {
    if game.source != bigame_core::games::Source::Steam {
        if let Some((program, args)) = game.launch_command.as_ref().and_then(|c| c.split_first()) {
            return Some(game_card::Launch::Direct(program.clone(), args.to_vec()));
        }
    }
    bigame_core::launchers::Start::for_game(game).map(game_card::Launch::Through)
}

/// Start `entry`'s game: with Big Game Mode's launch settings (Gamescope, Wine
/// FSR, vkBasalt, frame generation) on top of its profile when Big Game Mode
/// starts it, or through its launcher.
fn launch_game(entry: &game_card::Entry, anchor: &gtk4::Widget) {
    match entry.launch.clone() {
        Some(game_card::Launch::Direct(program, args)) => {
            launch_directly(entry, program, args, anchor);
        }
        Some(game_card::Launch::Through(start)) => launch_through(entry, start, anchor),
        None => {}
    }
}

/// Start a game Big Game Mode runs itself, its launch settings around it.
fn launch_directly(
    entry: &game_card::Entry,
    program: String,
    args: Vec<String>,
    anchor: &gtk4::Widget,
) {
    let exe = entry.key.clone();
    let title = entry.title.clone();
    let anchor = anchor.clone();
    glib::spawn_future_local(async move {
        let exe_for_launch = exe.clone();
        let title_for_log = title.clone();
        let result = gio::spawn_blocking(move || {
            let profile = bigame_core::profiles::load(&exe_for_launch).ok();
            let gs_mode = profile
                .as_ref()
                .map_or(bigame_core::gamescope::Mode::Auto, |p| p.gamescope_mode);
            let gs_cfg = profile.and_then(|p| p.gamescope);
            tracing::info!(
                game = %title_for_log,
                profile = %exe_for_launch,
                launch_program = %program,
                launch_args = ?args,
                "launch requested from Profiles"
            );
            let video = bigame_core::video_config::load();
            bigame_core::launcher::LaunchPlan::build_for_game(
                &program,
                &args,
                &exe_for_launch,
                &video,
                gs_cfg.as_ref(),
                gs_mode,
            )
            .spawn()
            .map(|mut child| {
                // Reaped off the UI thread: a dropped handle would leave an
                // exited Gamescope a zombie for the life of the UI, and a
                // zombie still matches "is it running".
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            })
        })
        .await;
        match result {
            Ok(Ok(())) => {
                tracing::info!(game = %title, "launch succeeded");
                toast::show(
                    &anchor,
                    &i18n("%s started with Big Game Mode's launch settings").replace("%s", &title),
                );
            }
            Ok(Err(e)) => launch_failed(&anchor, &title, &error_text(&e)),
            Err(_) => launch_failed(&anchor, &title, ""),
        }
    });
}

/// Ask a game's launcher to start it. The launcher starts the game in its
/// own process tree: falcond's profile reaches the game — while Turbo is on
/// — but nothing Big Game Mode wraps a game with does, and the toast says both
/// rather than promising the launch settings.
fn launch_through(
    entry: &game_card::Entry,
    start: bigame_core::launchers::Start,
    anchor: &gtk4::Widget,
) {
    let title = entry.title.clone();
    let anchor = anchor.clone();
    glib::spawn_future_local(async move {
        let by = start.by;
        tracing::info!(game = %title, launcher = by, argv = ?start.argv, "launch through the launcher requested from Profiles");
        let result = gio::spawn_blocking(move || {
            let unit = bigame_core::systemd::Reader::shared()
                .and_then(|r| r.unit_state(bigame_core::turbo::BACKEND_UNIT))
                .filter(bigame_core::systemd::UnitState::is_installed);
            let falcond = unit.is_some();
            let turbo_on = unit.is_some_and(|u| u.is_active());
            start.spawn().map(|()| (falcond, turbo_on))
        })
        .await;
        match result {
            Ok(Ok((falcond, turbo_on))) => {
                tracing::info!(game = %title, launcher = by, falcond, turbo_on, "launcher asked to start the game");
                let text = if !falcond {
                    i18n(
                        "Asked %l to start %s. falcond is not installed, so no game profile applies.",
                    )
                } else if !turbo_on {
                    i18n(
                        "Asked %l to start %s. Turbo is off, so its profile does not apply until Turbo is switched on in Home.",
                    )
                } else if by == "Steam" {
                    i18n(
                        "Asked Steam to start %s. Its profile applies; the launch settings reach a Steam game only through Steam's launch options.",
                    )
                } else if by == "Heroic" {
                    i18n(
                        "Asked Heroic to start %s. Its profile applies; the game's own launch settings reach it only through its settings in Heroic, written when its profile is saved.",
                    )
                } else {
                    i18n(
                        "Asked %l to start %s. Its profile applies; Big Game Mode's launch settings do not reach a game its launcher starts.",
                    )
                };
                toast::show(&anchor, &text.replace("%l", by).replace("%s", &title));
            }
            Ok(Err(e)) => launch_failed(&anchor, &title, &error_text(&e)),
            Err(_) => launch_failed(&anchor, &title, ""),
        }
    });
}

/// Say a launch failed, and why.
fn launch_failed(anchor: &gtk4::Widget, title: &str, why: &str) {
    tracing::error!(game = %title, error = %why, "launch failed");
    toast::show(
        anchor,
        &i18n("Could not start %t: %e")
            .replace("%t", title)
            .replace("%e", why),
    );
}

/// Open a card's profile: the existing file for editing, or a new profile
/// keyed on the game's real process name, so falcond can match it.
fn open_profile(entry: &game_card::Entry, nav: &adw::NavigationView) {
    match &entry.profile_stem {
        Some(stem) => nav.push(&build_detail_page(stem, Some(entry))),
        None => nav.push(&build_new_page(&entry.key, Some(entry))),
    }
}

/// Where a game came from, for display: launchers by their names, a game from
/// the application menu in the user's language.
pub(crate) fn source_label(source: bigame_core::games::Source) -> String {
    match source {
        bigame_core::games::Source::Native => i18n("Native"),
        other => other.label().to_owned(),
    }
}

/// Overflow menu for one card.
// One action per entry, built where the menu is; splitting them would
// separate an item from what it does.
#[allow(clippy::too_many_lines)]
fn show_card_menu(
    entry: &game_card::Entry,
    anchor: &gtk4::Widget,
    nav: &adw::NavigationView,
    library: &std::rc::Weak<LibraryView>,
) {
    // What the card shows changed: the library is read again.
    let rescan = {
        let library = library.clone();
        move || {
            if let Some(view) = library.upgrade() {
                refresh_library(&view);
            }
        }
    };
    // Only what applies to this game, in the order a person needs it:
    // start it, set it up, look inside it, measure it, undo.
    let menu = gio::Menu::new();
    if entry.launch.is_some() {
        menu.append(Some(&i18n("Launch (Turbo)")), Some("card.launch"));
    }
    if entry.has_profile {
        menu.append(Some(&i18n("Edit profile")), Some("card.edit"));
    } else {
        menu.append(Some(&i18n("Create with Wizard")), Some("card.wizard"));
        menu.append(Some(&i18n("Create profile")), Some("card.edit"));
    }
    if entry.target.is_some() {
        menu.append(Some(&i18n("AI Graphics…")), Some("card.ai"));
    }
    // Measuring needs a handle on the game's own process, which only exists
    // for games that start without a launcher.
    if entry.launch_command.is_some() {
        menu.append(Some(&i18n("Measure the difference")), Some("card.measure"));
    }
    if entry.ai_installed {
        menu.append(
            Some(&i18n("Restore the game's graphics")),
            Some("card.restore"),
        );
    }
    if entry.has_profile && !entry.system_profile {
        menu.append(Some(&i18n("Delete profile")), Some("card.delete"));
    }

    let group = gio::SimpleActionGroup::new();

    if entry.launch.is_some() {
        let launch = gio::SimpleAction::new("launch", None);
        let entry = entry.clone();
        let anchor = anchor.clone();
        launch.connect_activate(move |_, _| launch_game(&entry, &anchor));
        group.add_action(&launch);
    }

    if !entry.has_profile {
        let wizard = gio::SimpleAction::new("wizard", None);
        let key = entry.key.clone();
        let anchor = anchor.clone();
        let rescan = rescan.clone();
        wizard.connect_activate(move |_, _| {
            let anchor_saved = anchor.clone();
            let rescan = rescan.clone();
            crate::views::profile_wizard::open_with_suggested_name(&anchor, &key, move |_| {
                toast::show(&anchor_saved, &i18n("Profile created"));
                rescan();
            });
        });
        group.add_action(&wizard);
    }

    if let Some(target) = entry.target.clone().filter(|_| entry.ai_installed) {
        let restore = gio::SimpleAction::new("restore", None);
        let anchor = anchor.clone();
        let rescan = rescan.clone();
        restore.connect_activate(move |_, _| {
            let anchor = anchor.clone();
            let target = target.clone();
            let rescan = rescan.clone();
            glib::spawn_future_local(async move {
                let t = target.clone();
                let result = gio::spawn_blocking(move || bigame_core::graphics::remove(&t)).await;
                toast::show(
                    &anchor,
                    &match result {
                        Ok(Ok(_)) => i18n("The game's files are as they were before"),
                        Ok(Err(e)) => format!("{}: {}", i18n("Could not restore"), error_text(&e)),
                        Err(_) => i18n("Could not restore"),
                    },
                );
                rescan();
            });
        });
        group.add_action(&restore);
    }

    if let Some(command) = entry.launch_command.clone() {
        let measure = gio::SimpleAction::new("measure", None);
        let title = entry.title.clone();
        let anchor = anchor.clone();
        measure.connect_activate(move |_, _| {
            crate::views::measure_dialog::present(&anchor, &title, &command);
        });
        group.add_action(&measure);
    }

    if let Some(target) = entry.target.clone() {
        let ai = gio::SimpleAction::new("ai", None);
        let anchor = anchor.clone();
        ai.connect_activate(move |_, _| {
            crate::views::ai_graphics::open(&anchor, target.clone(), None);
        });
        group.add_action(&ai);
    }

    let edit = gio::SimpleAction::new("edit", None);
    {
        let nav = nav.clone();
        let entry = entry.clone();
        edit.connect_activate(move |_, _| open_profile(&entry, &nav));
    }
    group.add_action(&edit);

    if let Some(stem) = entry.profile_stem.clone().filter(|_| !entry.system_profile) {
        let delete = gio::SimpleAction::new("delete", None);
        let entry = entry.clone();
        let anchor_ref = anchor.clone();
        let rescan = rescan.clone();
        delete.connect_activate(move |_, _| {
            let dialog = adw::AlertDialog::new(
                Some(&i18n("Delete this profile?")),
                Some(
                    &i18n("The profile for %s will be removed. This cannot be undone.")
                        .replace("%s", &entry.title),
                ),
            );
            dialog.add_response("cancel", &i18n("Cancel"));
            dialog.add_response("delete", &i18n("Delete"));
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");

            let stem = stem.clone();
            let anchor_inner = anchor_ref.clone();
            let rescan = rescan.clone();
            dialog.connect_response(None, move |_, response| {
                if response != "delete" {
                    return;
                }
                let stem = stem.clone();
                let anchor = anchor_inner.clone();
                let rescan = rescan.clone();
                // The helper's D-Bus call waits for Polkit's password: off
                // the main thread, or the window freezes until it is typed.
                glib::spawn_future_local(async move {
                    let result =
                        gio::spawn_blocking(move || bigame_core::profiles::delete(&stem)).await;
                    match result {
                        Ok(Ok(())) => {
                            toast::show(&anchor, &i18n("Profile deleted"));
                            rescan();
                        }
                        Ok(Err(e)) => toast::show(
                            &anchor,
                            &i18n("Could not delete profile: %s").replace("%s", &error_text(&e)),
                        ),
                        Err(_) => toast::show(&anchor, &i18n("Could not delete profile")),
                    }
                });
            });
            dialog.present(Some(&anchor_ref));
        });
        group.add_action(&delete);
    }

    let popover = gtk4::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(anchor);
    popover.insert_action_group("card", Some(&group));
    // `closed` is emitted before the chosen item's action is activated;
    // unparenting right away would detach the popover — and the "card"
    // actions inserted on it — first, so no item would do anything. Let the
    // activation run, then unparent.
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
}
