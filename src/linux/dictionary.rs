//! The dictionary window: a field to add a word, the list to remove one from.
//! A regular toplevel and not a layer surface, since only a toplevel takes
//! keyboard focus on Wayland; Hyprland floats it by title (`hypr`).
//!
//! The file stays the store: the list is read again each time the window is
//! presented, so a word added from a card or typed into the file shows.

use super::hypr::DICTIONARY_TITLE;
use crate::surface::dictionary as text;
use crate::surface::theme::{self, Face, Palette};
use gramly_core::config;
use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static OPEN: RefCell<Option<Rc<View>>> = const { RefCell::new(None) };
}

struct View {
    window: gtk::Window,
    entry: gtk::Entry,
    list: gtk::Box,
    count: gtk::Label,
    changed: Box<dyn Fn()>,
}

/// Opens the window, or brings the open one forward, with the list read
/// again. `changed` runs after a word is added or removed, so the text on
/// screen is checked against the new list.
pub fn show(changed: impl Fn() + 'static) {
    let view = OPEN.with_borrow_mut(|open| {
        Rc::clone(open.get_or_insert_with(|| View::new(Box::new(changed))))
    });
    view.reload();
    view.window.present();
    view.entry.grab_focus();
}

fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class(class);
    label.set_xalign(0.0);
    label
}

fn cross(tooltip: &str) -> gtk::Button {
    let button = gtk::Button::with_label("×");
    button.add_css_class("gramly-cross");
    button.set_focusable(false);
    button.set_tooltip_text(Some(tooltip));
    button
}

impl View {
    fn new(changed: Box<dyn Fn()>) -> Rc<Self> {
        let pad = theme::DICTIONARY_PAD as i32;
        let title = label(text::TITLE, "gramly-dictionary-title");
        title.set_hexpand(true);
        let close = cross("Close");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.append(&title);
        header.append(&close);

        let entry = gtk::Entry::builder()
            .placeholder_text(text::PLACEHOLDER)
            .hexpand(true)
            .height_request(theme::BUTTON_HEIGHT as i32)
            .build();
        entry.add_css_class("gramly-field");
        let add = gtk::Button::with_label(text::ADD);
        add.add_css_class("gramly-button");
        add.add_css_class("gramly-plain");
        add.set_focusable(false);
        add.set_height_request(theme::BUTTON_HEIGHT as i32);
        let form = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        form.append(&entry);
        form.append(&add);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build();
        let count = label("", "gramly-dictionary-count");

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(pad)
            .margin_bottom(pad)
            .margin_start(pad)
            .margin_end(pad)
            .build();
        body.append(&header);
        body.append(&label(text::SUBTITLE, "gramly-dictionary-hint"));
        body.append(&form);
        body.append(&scroll);
        body.append(&count);

        let window = gtk::Window::builder()
            .title(DICTIONARY_TITLE)
            .decorated(false)
            .resizable(false)
            .default_width(theme::DICTIONARY_WIDTH as i32)
            .default_height(theme::DICTIONARY_HEIGHT as i32)
            .child(&body)
            .build();
        window.add_css_class("gramly-dictionary");

        let view = Rc::new(Self {
            window,
            entry,
            list,
            count,
            changed,
        });
        view.window.connect_close_request(|_| {
            OPEN.set(None);
            gtk::glib::Propagation::Proceed
        });
        close.connect_clicked({
            let window = view.window.clone();
            move |_| window.close()
        });
        let escape = gtk::EventControllerKey::new();
        escape.connect_key_pressed({
            let window = view.window.clone();
            move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    window.close();
                }
                gtk::glib::Propagation::Proceed
            }
        });
        view.window.add_controller(escape);
        let weak = Rc::downgrade(&view);
        add.connect_clicked({
            let weak = weak.clone();
            move |_| weak.upgrade().iter().for_each(|view| view.add())
        });
        view.entry
            .connect_activate(move |_| weak.upgrade().iter().for_each(|view| view.add()));
        view
    }

    fn add(self: &Rc<Self>) {
        let input = self.entry.text();
        self.entry.set_text("");
        match config::learn_word(&input) {
            Ok(true) => (self.changed)(),
            Ok(false) => {}
            Err(why) => note!("adding {input:?} to the dictionary: {why:#}"),
        }
        self.reload();
    }

    fn remove(self: &Rc<Self>, word: &str) {
        match config::forget_word(word) {
            Ok(true) => (self.changed)(),
            Ok(false) => {}
            Err(why) => note!("removing {word:?} from the dictionary: {why:#}"),
        }
        self.reload();
    }

    fn reload(self: &Rc<Self>) {
        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }
        let words = config::sorted_dictionary();
        if words.is_empty() {
            let empty = label(text::EMPTY, "gramly-dictionary-hint");
            empty.set_wrap(true);
            empty.set_justify(gtk::Justification::Center);
            empty.set_margin_top(28);
            self.list.append(&empty);
        }
        for word in &words {
            let remove = cross(&format!("Remove {word}"));
            let weak = Rc::downgrade(self);
            let target = word.clone();
            remove.connect_clicked(move |_| weak.upgrade().iter().for_each(|v| v.remove(&target)));
            let name = label(word, "gramly-dictionary-word");
            name.set_hexpand(true);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.set_height_request(theme::DICTIONARY_ROW as i32);
            row.append(&name);
            row.append(&remove);
            self.list.append(&row);
        }
        self.count.set_text(&text::count(words.len()));
    }
}

fn font(face: Face) -> String {
    format!(
        "font-size: {}px; font-weight: {};",
        face.size,
        face.weight.css()
    )
}

/// Appended to the card's stylesheet, which is at the user level and so
/// beats a matugen theme here too.
pub fn css(p: &Palette) -> String {
    format!(
        "window.gramly-dictionary, window.gramly-dictionary * {{ font-family: \"{family}\", sans-serif; }}
         window.gramly-dictionary {{ background-color: {surface}; border: {edge}px solid {edge_color}; border-radius: {corner}px; box-shadow: none; }}
         .gramly-dictionary-title {{ color: {text}; {title} }}
         .gramly-dictionary-hint {{ color: {text2}; {hint} }}
         .gramly-dictionary-count {{ color: {text2}; {count} }}
         .gramly-dictionary-word {{ color: {text}; {word} }}
         entry.gramly-field {{ min-height: 0; padding: 0 10px; background: none; background-color: transparent; border: 1px solid {line}; border-radius: {control}px; box-shadow: none; outline: none; color: {text}; caret-color: {text}; {word} }}
         entry.gramly-field text placeholder {{ color: {text2}; }}
         button.gramly-cross {{ all: unset; padding: 0 8px; border-radius: {control}px; }}
         button.gramly-cross label {{ color: {text2}; font-size: 18px; }}
         button.gramly-cross:hover {{ background-color: {fill}; }}
         button.gramly-cross:hover label {{ color: {text}; }}
         scrolledwindow undershoot, scrolledwindow overshoot {{ background: none; }}",
        family = theme::FONT_FAMILY,
        surface = p.surface.css(),
        edge = super::card::edge_width(),
        edge_color = p.edge.css(),
        corner = theme::CORNER,
        control = theme::CORNER_CONTROL,
        text = p.text.css(),
        text2 = p.text2.css(),
        line = p.line.css(),
        fill = p.fill.css(),
        title = font(theme::DICTIONARY_TITLE),
        hint = font(theme::DICTIONARY_HINT),
        count = font(theme::DICTIONARY_COUNT),
        word = font(theme::DICTIONARY_WORD),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stylesheet_is_the_palette_and_the_measures() {
        for p in [&theme::LIGHT, &theme::DARK] {
            let css = css(p);
            for color in [p.surface, p.edge, p.text, p.text2, p.line, p.fill] {
                assert!(css.contains(&color.css()));
            }
            assert!(css.contains("font-size: 15px; font-weight: 600"));
            assert!(css.contains("font-size: 12.5px; font-weight: 400"));
        }
    }
}
