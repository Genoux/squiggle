//! The suggestion card as GTK widgets, sized and coloured like the macOS
//! one (`src/macos/overlay.rs`). Rows are flat buttons: the whole row is
//! the target, and `:hover` lights it because the card is a real surface
//! that gets pointer events.

use crate::surface::theme::{self, Rgba};
use crate::surface::{Action, Bit, Card};
use gramly_core::Kind;
use gtk::prelude::*;
use gtk4 as gtk;
use std::rc::Rc;

/// Space between the card's edge and its rows.
const EDGE_PAD: i32 = 6;
/// Space inside a row; title, replacement and the action labels all start
/// at this inset so they share one left edge.
const ROW_PAD: i32 = 12;
const CARD_WIDTH: i32 = 380;
const BUTTON_HEIGHT: i32 = 34;
const SENTENCE_LINES: i32 = 5;

const fn rgb(r: f64, g: f64, b: f64) -> Rgba {
    Rgba { r, g, b, a: 1.0 }
}

// Flow's palette, which this card keeps until it takes the Ink look from
// `surface::theme`.
const SURFACE: Rgba = rgb(0.086, 0.090, 0.098);
const EDGE: Rgba = rgb(0.157, 0.165, 0.173);
const TEXT: Rgba = rgb(0.925, 0.929, 0.937);
const MUTED: Rgba = rgb(0.549, 0.565, 0.580);
const ACCENT: Rgba = rgb(0.180, 0.835, 0.451);
const HOVER: Rgba = rgb(0.153, 0.157, 0.169);
const ACCENT_HOVER: Rgba = rgb(0.290, 0.890, 0.553);
const ON_ACCENT: Rgba = rgb(0.078, 0.082, 0.059);

pub type OnAction = Rc<dyn Fn(Action, usize)>;

/// A card ready to be shown: its widgets, its size in logical pixels, and
/// its clickable rows for `GRAMLY_STATE`.
pub struct Built {
    pub root: gtk::Box,
    pub size: (f64, f64),
    pub rows: Vec<(Action, gtk::Widget)>,
}

pub fn css() -> String {
    format!(
        "window.gramly {{ background: transparent; box-shadow: none; }}
         .gramly-card {{ background-color: {surface}; border: 1px solid {edge}; border-radius: 12px; }}
         button.gramly-row {{ all: unset; border-radius: 8px; }}
         button.gramly-row:hover {{ background-color: {hover}; }}
         button.gramly-row.gramly-filled {{ background-color: {accent}; padding: 0 16px; }}
         button.gramly-row.gramly-filled:hover {{ background-color: {accent_hover}; }}
         button.gramly-row.gramly-plain {{ padding: 0 14px; }}
         .gramly-title {{ color: {muted}; font-size: 13px; font-weight: 400; }}
         .gramly-action {{ color: {text}; font-size: 14px; font-weight: 500; }}
         .gramly-filled .gramly-action {{ color: {on_accent}; font-weight: 600; }}
         .gramly-sentence {{ color: {text}; font-size: 15px; }}",
        surface = SURFACE.css(),
        edge = EDGE.css(),
        hover = HOVER.css(),
        accent = ACCENT.css(),
        accent_hover = ACCENT_HOVER.css(),
        muted = MUTED.css(),
        text = TEXT.css(),
        on_accent = ON_ACCENT.css(),
    )
}

/// The sentence with its changes in place: what a fix takes out struck, in
/// red (blue for clearer wording), what it puts there in green.
fn sentence_markup(bits: &[Bit], kind: Kind) -> String {
    let gone = if kind == Kind::Clarity {
        theme::ADVICE
    } else {
        theme::MISTAKE
    };
    bits.iter()
        .map(|bit| match bit {
            Bit::Same(text) => gtk::glib::markup_escape_text(text).to_string(),
            Bit::Gone(text) => format!(
                r#"<span foreground="{}" strikethrough="true">{}</span>"#,
                gone.hex(),
                gtk::glib::markup_escape_text(text)
            ),
            Bit::New(text) => format!(
                r#"<span foreground="{}" weight="600">{}</span>"#,
                ACCENT.hex(),
                gtk::glib::markup_escape_text(text)
            ),
        })
        .collect()
}

pub fn build(card: &Card, index: usize, on_action: &OnAction) -> Built {
    let mut rows = Vec::new();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("gramly-card");
    let width = layout(&root, card, &mut Rows::new(index, on_action, &mut rows));
    let (_, height, ..) = root.measure(gtk::Orientation::Vertical, width);
    root.set_size_request(width, height);
    Built {
        root,
        size: (f64::from(width), f64::from(height)),
        rows,
    }
}

/// Where a card's rows are collected and what their clicks call.
struct Rows<'a> {
    index: usize,
    on_action: &'a OnAction,
    found: &'a mut Vec<(Action, gtk::Widget)>,
}

impl<'a> Rows<'a> {
    fn new(
        index: usize,
        on_action: &'a OnAction,
        found: &'a mut Vec<(Action, gtk::Widget)>,
    ) -> Self {
        Self {
            index,
            on_action,
            found,
        }
    }

    fn button(&mut self, action: Action, height: i32, class: &str) -> gtk::Button {
        let button = gtk::Button::new();
        button.set_focusable(false);
        button.set_focus_on_click(false);
        button.set_height_request(height);
        button.add_css_class("gramly-row");
        button.add_css_class(class);
        let (on_action, index) = (Rc::clone(self.on_action), self.index);
        button.connect_clicked(move |_| on_action(action, index));
        self.found.push((action, button.clone().upcast()));
        button
    }
}

fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_valign(gtk::Align::Center);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.add_css_class(class);
    label
}

/// The title, the sentence with every change in place, then a button per
/// action.
fn layout(root: &gtk::Box, card: &Card, rows: &mut Rows) -> i32 {
    let pad = EDGE_PAD + ROW_PAD;
    let heading = label(&card.title, "gramly-title");
    let body = gtk::Label::new(None);
    body.set_markup(&sentence_markup(&card.bits, card.kind));
    body.add_css_class("gramly-sentence");
    body.set_xalign(0.0);
    body.set_wrap(true);
    // A wrapping label's natural width is its whole text on one line, and the
    // layer surface took that width; capped, the card's width decides.
    body.set_max_width_chars(1);
    body.set_lines(SENTENCE_LINES);
    body.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_margin_top(16);
    for (position, &action) in card.actions.iter().enumerate() {
        let button = rows.button(
            action,
            BUTTON_HEIGHT,
            if position == 0 {
                "gramly-filled"
            } else {
                "gramly-plain"
            },
        );
        button.set_child(Some(&label(action.label(), "gramly-action")));
        buttons.append(&button);
    }

    let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
    column.set_margin_top(pad);
    column.set_margin_start(pad);
    column.set_margin_end(pad);
    column.set_margin_bottom(EDGE_PAD + 8);
    column.append(&heading);
    column.append(&body);
    column.append(&buttons);
    root.append(&column);
    CARD_WIDTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sentence_marks_what_goes_and_what_comes() {
        let bits = [
            Bit::Same("I ".into()),
            Bit::Gone("goed".into()),
            Bit::New("went".into()),
        ];
        assert_eq!(
            sentence_markup(&bits, Kind::Correctness),
            format!(
                r##"I <span foreground="{}" strikethrough="true">goed</span><span foreground="{}" weight="600">went</span>"##,
                theme::MISTAKE.hex(),
                ACCENT.hex()
            )
        );
    }

    #[test]
    fn clearer_wording_strikes_in_blue() {
        let bits = [Bit::Gone("a".into())];
        assert!(sentence_markup(&bits, Kind::Clarity).contains(&theme::ADVICE.hex()));
    }

    #[test]
    fn markup_characters_in_the_text_are_escaped() {
        assert_eq!(
            sentence_markup(&[Bit::Same("a < b & c".into())], Kind::Correctness),
            "a &lt; b &amp; c"
        );
    }

    #[test]
    fn the_stylesheet_carries_the_palette() {
        let css = css();
        assert!(css.contains(&ACCENT.css()));
        assert!(css.contains(&HOVER.css()));
        assert!(css.contains("border-radius: 12px"));
    }
}
