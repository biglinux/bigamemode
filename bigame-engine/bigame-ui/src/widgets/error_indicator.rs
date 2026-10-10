// The stored callbacks are boxed closures over GTK widgets; naming each
// shape with a type alias would add indirection without making the
// signatures easier to read. The wide setter takes one argument per piece
// of the error banner it fills in.
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::many_single_char_names
)]
use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

/// A button that shows a prominent error indicator when something is wrong.
pub struct ErrorIndicator {
    button: gtk4::Button,
    error_title: std::sync::Arc<std::sync::Mutex<String>>,
    error_msg: std::sync::Arc<std::sync::Mutex<String>>,
    solution: std::sync::Arc<std::sync::Mutex<String>>,
    /// Optional action: (`button_label`, `shell_command_args`).
    action: std::sync::Arc<std::sync::Mutex<Option<(String, Vec<String>)>>>,
    /// Optional copy action: (`button_label`, `text_to_copy`).
    copy_action: std::sync::Arc<std::sync::Mutex<Option<(String, String)>>>,
    /// Called once the action's command has ended, however it ended.
    action_done: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

impl ErrorIndicator {
    pub fn new() -> Self {
        let button = gtk4::Button::builder()
            .icon_name("dialog-information-symbolic")
            .css_classes(["error-indicator", "circular"])
            .tooltip_text(i18n("Service Issues Detected"))
            .visible(false)
            .build();

        let error_title = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let error_msg = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let solution = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let action: std::sync::Arc<std::sync::Mutex<Option<(String, Vec<String>)>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let copy_action: std::sync::Arc<std::sync::Mutex<Option<(String, String)>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));

        let t = error_title.clone();
        let m = error_msg.clone();
        let s = solution.clone();
        let a = action.clone();
        let c = copy_action.clone();
        let action_done: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::default();
        let done = Rc::clone(&action_done);

        button.connect_clicked(move |btn| {
            let win = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
            let title = t.lock().unwrap().clone();
            let msg = m.lock().unwrap().clone();
            let sol = s.lock().unwrap().clone();
            let act = a.lock().unwrap().clone();
            let copy_act = c.lock().unwrap().clone();

            let dialog = adw::AlertDialog::builder()
                .heading(title)
                .body(format!(
                    "{}\n\n<b>{}</b>\n{}",
                    glib::markup_escape_text(&msg),
                    i18n("What to do"),
                    glib::markup_escape_text(&sol)
                ))
                .body_use_markup(true)
                .close_response("close")
                .default_response("close")
                .build();

            dialog.add_response("close", &i18n("Close"));

            if let Some((ref label, _)) = act {
                dialog.add_response("action", label);
                dialog.set_response_appearance("action", adw::ResponseAppearance::Suggested);
            }
            if let Some((ref label, _)) = copy_act {
                dialog.add_response("copy", label);
            }

            if act.is_some() || copy_act.is_some() {
                let (anchor, done) = (btn.clone(), Rc::clone(&done));
                dialog.connect_response(None, move |_, response| {
                    if response == "action" {
                        if let Some((_, cmd)) = &act {
                            run_action(&anchor, cmd.clone(), done.borrow().clone());
                        }
                    } else if response == "copy"
                        && let Some((_, text)) = &copy_act
                        && let Some(display) = gtk4::gdk::Display::default()
                    {
                        display.clipboard().set_text(text);
                    }
                });
            }

            match win.as_ref() {
                Some(w) => dialog.present(Some(&w.clone())),
                None => dialog.present(None::<&gtk4::Window>),
            }
        });

        Self {
            button,
            error_title,
            error_msg,
            solution,
            action,
            copy_action,
            action_done,
        }
    }

    pub fn widget(&self) -> &gtk4::Button {
        &self.button
    }

    /// Call `f` once the action's command has ended, to read the state again.
    pub fn connect_action_done(&self, f: impl Fn() + 'static) {
        *self.action_done.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_error(&self, title: &str, msg: &str, solution: &str) {
        if let Ok(mut t) = self.error_title.lock() {
            *t = title.to_string();
        }
        if let Ok(mut m) = self.error_msg.lock() {
            *m = msg.to_string();
        }
        if let Ok(mut s) = self.solution.lock() {
            *s = solution.to_string();
        }
        if let Ok(mut a) = self.action.lock() {
            *a = None;
        }
        if let Ok(mut c) = self.copy_action.lock() {
            *c = None;
        }
        self.button.set_visible(true);
    }

    /// Set error with install action and optional copy-command action.
    pub fn set_error_with_action_and_copy(
        &self,
        title: &str,
        msg: &str,
        solution: &str,
        action_label: &str,
        cmd: Vec<String>,
        copy_label: &str,
        copy_text: &str,
    ) {
        if let Ok(mut t) = self.error_title.lock() {
            *t = title.to_string();
        }
        if let Ok(mut m) = self.error_msg.lock() {
            *m = msg.to_string();
        }
        if let Ok(mut s) = self.solution.lock() {
            *s = solution.to_string();
        }
        if let Ok(mut a) = self.action.lock() {
            *a = Some((action_label.to_string(), cmd));
        }
        if let Ok(mut c) = self.copy_action.lock() {
            *c = Some((copy_label.to_string(), copy_text.to_string()));
        }
        self.button.set_visible(true);
    }

    pub fn clear(&self) {
        self.button.set_visible(false);
        if let Ok(mut a) = self.action.lock() {
            *a = None;
        }
        if let Ok(mut c) = self.copy_action.lock() {
            *c = None;
        }
    }
}

/// Run the action's command off the main thread and say how it ended: an
/// install cancelled at the password prompt, a locked package database or a
/// conflict is a failure the user has to hear about.
fn run_action(anchor: &gtk4::Button, cmd: Vec<String>, done: Option<Rc<dyn Fn()>>) {
    let Some((prog, args)) = cmd.split_first().map(|(p, a)| (p.clone(), a.to_vec())) else {
        return;
    };
    let anchor = anchor.clone();
    glib::spawn_future_local(async move {
        let shown = cmd.join(" ");
        let ended =
            gio::spawn_blocking(move || std::process::Command::new(prog).args(args).status()).await;
        let failure = match ended {
            Ok(Ok(status)) if status.success() => None,
            Ok(Ok(status)) => Some(match status.code() {
                Some(code) => i18n("%s ended with exit code %d")
                    .replace("%s", &shown)
                    .replace("%d", &code.to_string()),
                None => i18n("%s was stopped by a signal").replace("%s", &shown),
            }),
            Ok(Err(e)) => Some(format!("{shown}: {e}")),
            Err(_) => Some(i18n("the worker thread stopped")),
        };
        if let Some(details) = failure {
            crate::widgets::toast::error(
                &anchor,
                &i18n("Could not install the missing packages"),
                &details,
            );
        }
        if let Some(done) = done {
            done();
        }
    });
}
