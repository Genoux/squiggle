//! The OpenRouter key window. A regular toplevel, not a layer surface: only
//! a toplevel takes keyboard focus on Wayland, and the key is pasted in.

use crate::surface::{parse_key, update_config};
use gtk::prelude::*;
use gtk4 as gtk;
use squiggle_core::Config;
use std::cell::RefCell;

thread_local! {
    static OPEN: RefCell<Option<gtk::Window>> = const { RefCell::new(None) };
}

/// Opens the window, or raises it if it is open. `saved` runs after the key
/// is written.
pub fn ask(saved: impl Fn() + 'static) {
    if let Some(window) = OPEN.with_borrow(Clone::clone) {
        window.present();
        return;
    }
    let window = build(saved);
    OPEN.set(Some(window.clone()));
    window.present();
}

fn build(saved: impl Fn() + 'static) -> gtk::Window {
    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .activates_default(true)
        .hexpand(true)
        .build();
    if let Some(key) = Config::load().unwrap_or_default().openrouter_key {
        entry.set_text(&key);
    }
    let refusal = gtk::Label::builder().xalign(0.0).visible(false).build();
    refusal.add_css_class("error");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label("Cancel");
    let buttons = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    buttons.append(&cancel);
    buttons.append(&save);
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    let help = gtk::Label::builder()
        .label(
            "Paste a key from openrouter.ai/keys. It is stored in ~/.config/squiggle/config.toml, readable only by you. Leave it empty to check on this computer only.",
        )
        .wrap(true)
        .max_width_chars(48)
        .xalign(0.0)
        .build();
    body.append(&help);
    body.append(&entry);
    body.append(&refusal);
    body.append(&buttons);
    let window = gtk::Window::builder()
        .title("OpenRouter key")
        .resizable(false)
        .child(&body)
        .default_widget(&save)
        .build();

    window.connect_close_request(|_| {
        OPEN.set(None);
        gtk::glib::Propagation::Proceed
    });
    cancel.connect_clicked({
        let window = window.clone();
        move |_| window.close()
    });
    save.connect_clicked({
        let window = window.clone();
        move |_| match parse_key(&entry.text()) {
            Ok(key) => {
                update_config(|config| config.openrouter_key = key);
                saved();
                window.close();
            }
            Err(message) => {
                refusal.set_text(message);
                refusal.set_visible(true);
            }
        }
    });
    window
}
