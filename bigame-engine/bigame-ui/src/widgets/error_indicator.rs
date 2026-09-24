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
use gtk4::glib;
use libadwaita as adw;

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
                dialog.connect_response(None, move |_, response| {
                    if response == "action" {
                        if let Some((_, cmd)) = &act {
                            if let Some((prog, args)) = cmd.split_first() {
                                if let Ok(mut child) =
                                    std::process::Command::new(prog).args(args).spawn()
                                {
                                    std::thread::spawn(move || {
                                        let _ = child.wait();
                                    });
                                }
                            }
                        }
                    } else if response == "copy" {
                        if let Some((_, text)) = &copy_act {
                            if let Some(display) = gtk4::gdk::Display::default() {
                                display.clipboard().set_text(text);
                            }
                        }
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
        }
    }

    pub fn widget(&self) -> &gtk4::Button {
        &self.button
    }

    #[allow(dead_code)]
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
