//! The computer's DNS server, next to the comparison that measures them.
//!
//! One row says which DNS servers the connection to the internet is set to,
//! or why they cannot be changed from here, and offers to put back the
//! settings from before Big Game Mode when it changed them. Each resolver the
//! comparison measured gets a button to make it the connection's first DNS
//! server — after a confirmation that says what changes and how to undo it.
//! The work is `bigame_core::dns_config`, through `NetworkManager` as the user.

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use std::cell::RefCell;
use std::net::IpAddr;
use std::rc::Rc;

use bigame_core::dns_config::{
    self, Availability, Connection, DnsSettings, Outcome, SystemResolver,
};
use bigame_core::network::Resolver;
use bigame_core::text::Text;

use crate::i18n::{error_text, i18n, tr};
use crate::widgets::{info, toast};

/// What was last read about the connection.
#[derive(Default)]
struct Known {
    /// The connection, when its DNS can be changed.
    connection: Option<Connection>,
    /// Its DNS settings then.
    settings: Option<DnsSettings>,
}

/// The row that shows the computer's DNS, and the buttons that change it.
pub struct ComputerDns {
    row: adw::ActionRow,
    restore: gtk4::Button,
    known: RefCell<Known>,
    /// The "Use" buttons on the measured resolvers, and their server.
    buttons: RefCell<Vec<(gtk4::Button, IpAddr)>>,
    /// What the last change was verified to do, shown under the row.
    note: RefCell<Option<String>>,
}

impl ComputerDns {
    /// The row, read in the background once it exists.
    pub fn new() -> Rc<Self> {
        let row = adw::ActionRow::builder()
            .title(i18n("This computer's DNS"))
            .subtitle(i18n("Checking…"))
            .build();
        let restore = gtk4::Button::builder()
            .label(i18n("Restore previous"))
            .valign(gtk4::Align::Center)
            .visible(false)
            .build();
        row.add_suffix(&restore);
        row.add_suffix(&info::button(
            &i18n("This computer's DNS"),
            &i18n(
                "The DNS servers of the network connection that carries your traffic, as NetworkManager has them. \"Use\" on a measured resolver makes it the first one, and the servers your router hands out are no longer used. Before the first change, the connection's DNS settings are saved in ~/.local/state/bigame-mode/network; \"Restore previous\" puts them back and removes the copy. A faster lookup does not lower the latency of a game once it has connected.",
            ),
        ));
        let this = Rc::new(Self {
            row,
            restore,
            known: RefCell::new(Known::default()),
            buttons: RefCell::new(Vec::new()),
            note: RefCell::new(None),
        });
        {
            let weak = Rc::downgrade(&this);
            this.restore.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.restore();
                }
            });
        }
        this.refresh();
        this
    }

    /// The row to place on the page.
    pub fn row(&self) -> &adw::ActionRow {
        &self.row
    }

    /// Forget the buttons of a comparison that is being measured again.
    pub fn clear_buttons(&self) {
        self.buttons.borrow_mut().clear();
    }

    /// A button making `resolver` the computer's DNS server, or `None` for an
    /// address that cannot serve as one from any network: loopback (a local
    /// cache such as systemd-resolved's stub) or IPv6 link-local.
    pub fn use_button(self: &Rc<Self>, resolver: &Resolver) -> Option<gtk4::Button> {
        let address = resolver.address;
        let usable = match address {
            IpAddr::V4(a) => !a.is_loopback() && !a.is_unspecified(),
            IpAddr::V6(a) => !a.is_loopback() && !a.is_unspecified() && !a.is_unicast_link_local(),
        };
        if !usable {
            return None;
        }
        let button = gtk4::Button::builder()
            .label(i18n("Use"))
            .tooltip_text(i18n("Make it this computer's DNS server"))
            .valign(gtk4::Align::Center)
            .css_classes(["flat"])
            .build();
        let name = resolver.name.clone();
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |b| {
            if let Some(this) = weak.upgrade() {
                this.confirm(b, &name, address);
            }
        });
        self.buttons.borrow_mut().push((button.clone(), address));
        self.show_buttons();
        Some(button)
    }

    /// Read the connection and its settings again, off the main thread.
    fn refresh(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let read = gio::spawn_blocking(|| {
                let availability = dns_config::availability();
                let details = match &availability {
                    Availability::Ready(c) => {
                        Some((dns_config::current(c), dns_config::load_backup(&c.uuid)))
                    }
                    Availability::Unavailable(_) => None,
                };
                (availability, details)
            })
            .await;
            let Some(this) = weak.upgrade() else { return };
            let Ok((availability, details)) = read else {
                this.row
                    .set_subtitle(&i18n("Could not read the network settings"));
                return;
            };
            this.show(availability, details);
        });
    }

    #[allow(clippy::type_complexity)]
    fn show(
        &self,
        availability: Availability,
        details: Option<(
            anyhow::Result<DnsSettings>,
            anyhow::Result<Option<dns_config::Backup>>,
        )>,
    ) {
        let mut known = Known::default();
        self.restore.set_visible(false);
        self.restore.set_sensitive(true);
        match (availability, details) {
            (Availability::Unavailable(why), _) => {
                self.row.set_subtitle(&tr(&why));
            }
            (Availability::Ready(connection), Some((settings, backup))) => {
                let mut lines = vec![format!(
                    "{} ({}): {}",
                    connection.name,
                    connection.device,
                    settings.as_ref().map_or_else(error_text, servers_text)
                )];
                match backup {
                    Ok(Some(b)) => {
                        lines.push(
                            i18n("Big Game Mode set %s; the previous settings are kept")
                                .replace("%s", &b.applied),
                        );
                        if let Some(note) = self.note.borrow().as_ref() {
                            lines.push(note.clone());
                        }
                        self.restore.set_visible(true);
                    }
                    Ok(None) => {
                        self.note.borrow_mut().take();
                    }
                    Err(e) => lines.push(format!(
                        "{}: {}",
                        i18n("The backup of the previous settings cannot be read"),
                        error_text(&e)
                    )),
                }
                self.row.set_subtitle(&lines.join("\n"));
                known.settings = settings.ok();
                known.connection = Some(connection);
            }
            (Availability::Ready(_), None) => {}
        }
        *self.known.borrow_mut() = known;
        self.show_buttons();
    }

    /// Offer "Use" only where it can work, and not for the server already
    /// first with the automatic ones ignored.
    fn show_buttons(&self) {
        let known = self.known.borrow();
        for (button, address) in self.buttons.borrow().iter() {
            button.set_visible(known.connection.is_some());
            let in_use = known
                .settings
                .as_ref()
                .is_some_and(|s| is_primary(s, *address));
            button.set_sensitive(!in_use);
            button.set_label(&if in_use { i18n("In use") } else { i18n("Use") });
        }
    }

    /// Ask before changing the system's network settings.
    fn confirm(self: &Rc<Self>, button: &gtk4::Button, name: &str, server: IpAddr) {
        let Some(connection) = self.known.borrow().connection.clone() else {
            return;
        };
        let dialog = adw::AlertDialog::builder()
            .heading(i18n("Use %s as this computer's DNS?").replace("%s", name))
            .body(
                Text::fill(
                    &i18n(
                        "This changes the system's network settings for the connection “%s”: %s becomes its first DNS server, and the servers handed out automatically are no longer used. The current settings are saved first; “Restore previous” under Details › Network puts them back.",
                    ),
                    &[connection.name.clone(), server.to_string()],
                ),
            )
            .build();
        dialog.add_response("cancel", &i18n("Cancel"));
        dialog.add_response("apply", &i18n("Change DNS"));
        dialog.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let this = Rc::clone(self);
        let feedback = button.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "apply" {
                this.apply(&feedback, connection.clone(), server);
            }
        });
        dialog.present(Some(button));
    }

    fn apply(self: &Rc<Self>, feedback: &gtk4::Button, connection: Connection, server: IpAddr) {
        feedback.set_sensitive(false);
        let this = Rc::clone(self);
        let feedback = feedback.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || dns_config::apply(&connection, server)).await;
            let server = server.to_string();
            let text = match result {
                Ok(Ok(outcome)) => {
                    let (short, verified) = verification(&outcome, &server);
                    *this.note.borrow_mut() = Some(verified);
                    short
                }
                Ok(Err(e)) => format!("{}: {}", i18n("Could not change the DNS"), error_text(&e)),
                Err(_) => i18n("Could not change the DNS"),
            };
            toast::show(&feedback, &text);
            this.refresh();
        });
    }

    fn restore(self: &Rc<Self>) {
        let Some(connection) = self.known.borrow().connection.clone() else {
            return;
        };
        self.restore.set_sensitive(false);
        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || dns_config::restore(&connection)).await;
            let text = match result {
                Ok(Ok(Some(_))) => i18n("The previous DNS settings are back"),
                Ok(Ok(None)) => i18n("There was nothing to restore"),
                Ok(Err(e)) => format!(
                    "{}: {}",
                    i18n("Could not restore the previous DNS settings"),
                    error_text(&e)
                ),
                Err(_) => i18n("Could not restore the previous DNS settings"),
            };
            toast::show(&this.row, &text);
            this.refresh();
        });
    }
}

/// What an apply left in place: a short line for the toast and the one kept
/// under the row, both read back rather than assumed.
fn verification(outcome: &Outcome, server: &str) -> (String, String) {
    match &outcome.system {
        SystemResolver::Uses => {
            let text = i18n("%s is now this computer's DNS server").replace("%s", server);
            (text.clone(), text)
        }
        SystemResolver::Other(servers) => (
            i18n("%s is set, but another program decides the lookups").replace("%s", server),
            Text::fill(
                &i18n(
                    "NetworkManager uses %s, but this computer's lookups go to %s, which another program (a VPN, for example) sets in /etc/resolv.conf",
                ),
                &[server.to_owned(), servers.join(", ")],
            ),
        ),
        SystemResolver::Unknown if outcome.device_ok => {
            let text = i18n("%s is now this connection's DNS server").replace("%s", server);
            (text.clone(), text)
        }
        SystemResolver::Unknown => {
            let text = i18n(
                "%s was saved for the connection, but NetworkManager does not report it in use yet",
            )
            .replace("%s", server);
            (text.clone(), text)
        }
    }
}

/// Whether `server` is first in its family's list with the automatic ones
/// ignored: what "Use" would make it.
fn is_primary(settings: &DnsSettings, server: IpAddr) -> bool {
    let (list, ignore) = if server.is_ipv4() {
        (&settings.ipv4_dns, settings.ipv4_ignore_auto_dns)
    } else {
        (&settings.ipv6_dns, settings.ipv6_ignore_auto_dns)
    };
    ignore
        && list
            .first()
            .and_then(|e| e.split('#').next())
            .and_then(|a| a.parse::<IpAddr>().ok())
            == Some(server)
}

/// The servers a connection is set to, for its row.
fn servers_text(settings: &DnsSettings) -> String {
    let mut parts: Vec<String> = settings
        .ipv4_dns
        .iter()
        .chain(&settings.ipv6_dns)
        .cloned()
        .collect();
    // Automatic servers come from the network when it connects; they are
    // not in the profile, so they are named rather than listed.
    if !settings.ipv4_ignore_auto_dns {
        parts.push(i18n("automatic"));
    } else if !settings.ipv6_ignore_auto_dns {
        parts.push(i18n("automatic IPv6"));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_is_primary_only_when_first_with_automatic_ones_ignored() {
        let mut s = DnsSettings {
            ipv4_dns: vec!["1.1.1.1".into(), "1.0.0.1".into()],
            ..DnsSettings::default()
        };
        let cf: IpAddr = "1.1.1.1".parse().unwrap();
        assert!(!is_primary(&s, cf), "DHCP's servers still come along");
        s.ipv4_ignore_auto_dns = true;
        assert!(is_primary(&s, cf));
        assert!(!is_primary(&s, "1.0.0.1".parse().unwrap()));
        s.ipv4_dns[0] = "1.1.1.1#cloudflare-dns.com".into();
        assert!(is_primary(&s, cf));
        assert!(!is_primary(&s, "2606:4700:4700::1111".parse().unwrap()));
    }
}
