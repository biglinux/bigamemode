//! The page layout every page shares, the one Telemetry set: a wide page,
//! section titles, and cards with an icon in their header.
//!
//! [`Page`] is a scrolled, clamped column (up to 1280 px) whose rows of
//! cards turn into columns as the window narrows ([`Page::columns`],
//! [`Page::row`]); only the window's width changes the grid, never what the
//! cards show. Settings groups stay `AdwPreferencesGroup`s: the page draws
//! each as a card (`.bgm-cards` in the style sheet) with its rows inside,
//! and [`icon`] puts the group's icon before its title.

use adw::prelude::*;
use libadwaita as adw;

/// Below this width (sp) columns of settings rows go one under the other:
/// a row needs about 480 px for its title, its text and its control.
const ROWS_STACKED: f64 = 1000.0;
/// Below this width a row of small cards goes two by two.
const TWO_BY_TWO: f64 = 880.0;
/// Below this width a page's columns go one under the other.
const STACKED: f64 = 760.0;
/// Below this width everything is one column.
const ONE_COLUMN: f64 = 540.0;

/// A page of sections and cards.
#[derive(Clone)]
pub struct Page {
    bin: adw::BreakpointBin,
    content: gtk4::Box,
    rows_stacked: adw::Breakpoint,
    stacked: adw::Breakpoint,
    two_by_two: adw::Breakpoint,
    one_column: adw::Breakpoint,
}

impl Page {
    /// An empty page.
    #[must_use]
    pub fn new() -> Self {
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(24);
        content.set_margin_start(18);
        content.set_margin_end(18);
        content.add_css_class("bgm-cards");

        let clamp = adw::Clamp::builder()
            .maximum_size(1280)
            .tightening_threshold(1280)
            .child(&content)
            .build();
        let scroll = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .child(&clamp)
            .build();
        let bin = adw::BreakpointBin::builder()
            .width_request(300)
            .height_request(300)
            .child(&scroll)
            .build();

        // libadwaita applies the last breakpoint that matches; each repeats
        // the setters of the wider ones, added to all three as they come.
        let breakpoint = |width: f64| {
            let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                width,
                adw::LengthUnit::Sp,
            ));
            bin.add_breakpoint(bp.clone());
            bp
        };
        let rows_stacked = breakpoint(ROWS_STACKED);
        let two_by_two = breakpoint(TWO_BY_TWO);
        let stacked = breakpoint(STACKED);
        let one_column = breakpoint(ONE_COLUMN);
        Self {
            bin,
            content,
            rows_stacked,
            stacked,
            two_by_two,
            one_column,
        }
    }

    /// The page, to put in the window.
    #[must_use]
    pub fn widget(&self) -> &adw::BreakpointBin {
        &self.bin
    }

    /// Add `widget` across the whole width.
    pub fn add(&self, widget: &impl IsA<gtk4::Widget>) {
        self.content.append(widget);
    }

    /// A line under the title saying what the page is for.
    pub fn intro(&self, text: &str) -> gtk4::Label {
        let label = gtk4::Label::builder()
            .label(text)
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        self.add(&label);
        label
    }

    /// A section's title.
    pub fn section(&self, title: &str) -> gtk4::Label {
        let label = section(title);
        self.add(&label);
        label
    }

    /// Two columns of settings groups, side by side on a wide window and
    /// one under the other below [`ROWS_STACKED`]. Each is a vertical box to
    /// add groups to.
    #[must_use]
    pub fn columns(&self) -> (gtk4::Box, gtk4::Box) {
        let (row, left, right) = self.two_columns();
        let vertical = gtk4::Orientation::Vertical.to_value();
        let uneven = false.to_value();
        for bp in [
            &self.rows_stacked,
            &self.two_by_two,
            &self.stacked,
            &self.one_column,
        ] {
            bp.add_setter(&row, "orientation", Some(&vertical));
            // One under the other, each column only as tall as its groups.
            bp.add_setter(&row, "homogeneous", Some(&uneven));
        }
        (left, right)
    }

    /// Two columns of cards, which fit side by side down to [`STACKED`].
    #[must_use]
    pub fn card_columns(&self) -> (gtk4::Box, gtk4::Box) {
        let (row, left, right) = self.two_columns();
        let vertical = gtk4::Orientation::Vertical.to_value();
        let uneven = false.to_value();
        for bp in [&self.stacked, &self.one_column] {
            bp.add_setter(&row, "orientation", Some(&vertical));
            bp.add_setter(&row, "homogeneous", Some(&uneven));
        }
        (left, right)
    }

    fn two_columns(&self) -> (gtk4::Box, gtk4::Box, gtk4::Box) {
        let row = row_box();
        let column = || {
            let c = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
            c.set_valign(gtk4::Align::Start);
            row.append(&c);
            c
        };
        let (left, right) = (column(), column());
        self.add(&row);
        (row, left, right)
    }

    /// A row of equal cards: in a row when wide, two by two when they no
    /// longer fit (an even number of them), one under the other when narrow.
    pub fn row(&self, cards: &[&gtk4::Widget]) -> gtk4::Box {
        let row = row_box();
        if cards.len() >= 4 && cards.len().is_multiple_of(2) {
            let vertical = gtk4::Orientation::Vertical.to_value();
            for pair in cards.chunks(2) {
                let p = row_box();
                for c in pair {
                    p.append(*c);
                }
                self.one_column
                    .add_setter(&p, "orientation", Some(&vertical));
                row.append(&p);
            }
            for bp in [&self.two_by_two, &self.stacked, &self.one_column] {
                bp.add_setter(&row, "orientation", Some(&vertical));
            }
        } else {
            for c in cards {
                row.append(*c);
            }
            self.stack_when_narrow(&row);
        }
        self.add(&row);
        row
    }

    /// Turn `row` into a column below [`STACKED`].
    pub fn stack_when_narrow(&self, row: &gtk4::Box) {
        let vertical = gtk4::Orientation::Vertical.to_value();
        for bp in [&self.stacked, &self.one_column] {
            bp.add_setter(row, "orientation", Some(&vertical));
        }
    }

    /// Turn `row` into a column only below [`ONE_COLUMN`].
    pub fn stack_when_tiny(&self, row: &gtk4::Box) {
        let vertical = gtk4::Orientation::Vertical.to_value();
        self.one_column
            .add_setter(row, "orientation", Some(&vertical));
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new()
    }
}

/// A section's title, as on every page.
#[must_use]
pub fn section(title: &str) -> gtk4::Label {
    gtk4::Label::builder()
        .label(title)
        .xalign(0.0)
        .margin_top(6)
        .css_classes(["heading", "page-section"])
        .build()
}

/// A row of equal cards; a breakpoint turns it into a column.
#[must_use]
pub fn row_box() -> gtk4::Box {
    let b = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    b.set_homogeneous(true);
    b
}

/// A card, as Telemetry's: a small icon and title in its header.
#[must_use]
pub fn card(title: &str, icon: &str) -> gtk4::Box {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    root.add_css_class("card");
    root.add_css_class("telemetry-card");
    root.set_hexpand(true);
    root.update_property(&[gtk4::accessible::Property::Label(title)]);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let img = gtk4::Image::from_icon_name(icon);
    img.add_css_class("dim-label");
    header.append(&img);
    header.append(&fixed_label(title, &["caption-heading", "dim-label"]));
    root.append(&header);
    root
}

/// A card that holds controls rather than a figure: its icon and title in
/// the header, as a settings group's, with room under them.
#[must_use]
pub fn panel(title: &str, icon: &str) -> gtk4::Box {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    root.add_css_class("card");
    root.add_css_class("telemetry-card");
    root.set_hexpand(true);
    root.update_property(&[gtk4::accessible::Property::Label(title)]);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    let img = gtk4::Image::from_icon_name(icon);
    img.set_pixel_size(20);
    img.add_css_class("group-icon");
    header.append(&img);
    header.append(&fixed_label(title, &["heading"]));
    root.append(&header);
    root
}

/// A label of one line that never widens its card: a longer text is cut
/// short with an ellipsis.
#[must_use]
pub fn fixed_label(text: &str, classes: &[&str]) -> gtk4::Label {
    let label = gtk4::Label::builder()
        .label(text)
        .xalign(0.0)
        .lines(1)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .width_chars(1)
        .build();
    for c in classes {
        label.add_css_class(c);
    }
    label
}

/// Set `widget`, put in a settings group's card beside its rows, off the
/// card's edges as the rows are.
pub fn inset(widget: &impl IsA<gtk4::Widget>) {
    widget.set_margin_start(8);
    widget.set_margin_end(8);
    widget.set_margin_bottom(8);
}

/// A small rounded badge, e.g. a card's state.
#[must_use]
pub fn pill(text: &str) -> gtk4::Label {
    gtk4::Label::builder()
        .label(text)
        .valign(gtk4::Align::Center)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(22)
        .css_classes(["caption-heading", "telemetry-role", "telemetry-role-idle"])
        .build()
}

/// Put `icon` before the title of `group`, as a card's header has it.
///
/// The header is libadwaita's own (`box.header` of the group's template):
/// should it ever change shape, the group keeps its title without the icon.
pub fn icon(group: &adw::PreferencesGroup, icon: &str) {
    let header = group
        .first_child()
        .and_then(|b| b.first_child())
        .filter(|h| h.has_css_class("header"))
        .and_downcast::<gtk4::Box>();
    if let Some(header) = header {
        let img = gtk4::Image::from_icon_name(icon);
        img.set_pixel_size(20);
        img.set_valign(gtk4::Align::Start);
        img.add_css_class("group-icon");
        header.prepend(&img);
    }
}

/// A group with `title` and `icon`, drawn as a card on a [`Page`].
#[must_use]
pub fn group(title: &str, icon_name: &str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(title);
    icon(&g, icon_name);
    g
}
