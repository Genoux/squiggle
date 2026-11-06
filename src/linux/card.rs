//! The suggestion card as GTK widgets, laid out and coloured like the macOS
//! one (`src/macos/overlay.rs`), from the same values (`surface::theme`).
//! Buttons light under the pointer through `:hover`, because the card is a
//! real surface that gets pointer events.

use super::overlay::{fill, rounded};
use crate::surface::card::MENU;
use crate::surface::theme::{self, Face, Palette, Rgba, Shadow, Shape};
use crate::surface::{Action, Bit, Card, Rect};
use gtk::prelude::*;
use gtk4 as gtk;
use squiggle_core::Kind;
use std::ops::Range;
use std::rc::Rc;

pub type OnAction = Rc<dyn Fn(Action, usize)>;

/// A card ready to be shown: its widgets, its size in logical pixels, and
/// its clickable rows for `SQUIGGLE_STATE`.
pub struct Built {
    pub root: gtk::Box,
    pub rows: Vec<(Action, gtk::Widget)>,
    /// The ⋯ and its menu, hidden until the ⋯ is clicked.
    pub more: gtk::Button,
    pub menu: gtk::Box,
}

/// Sizes the card to its content and returns that size. Called once it is
/// in its window: the stylesheet's rules hang off the window, and measured
/// before, the sentence took the default font and the card came out 125px
/// short of what it then drew ("needs at least 353", every frame).
pub fn fit(root: &gtk::Box) -> (f64, f64) {
    let width = theme::CARD_WIDTH as i32;
    root.set_size_request(width, -1);
    // GTK measures a widget with its margins, which hold the shadow.
    let (left, right, top, bottom) = theme::SHADE;
    let across = width + (left + right) as i32;
    let (_, height, ..) = root.measure(gtk::Orientation::Vertical, across);
    let height = height - (top + bottom) as i32;
    root.set_size_request(width, height);
    (f64::from(width), f64::from(height))
}

fn font(face: Face) -> String {
    format!(
        "font-size: {}px; font-weight: {};",
        face.size,
        face.weight.css()
    )
}

/// GTK borders are whole pixels, so the hairline is one.
pub(super) fn edge_width() -> i32 {
    theme::EDGE_WIDTH.ceil() as i32
}

/// `0 - inset` and not `-inset`, which prints "-0" for none.
fn shadow(
    p: &Palette,
    Shadow {
        inset,
        drop,
        blur,
        share,
    }: Shadow,
) -> String {
    let alpha = f64::from(p.shadow) * share;
    format!(
        "0 {drop}px {blur}px {}px {}",
        0.0 - inset,
        Rgba {
            a: alpha,
            ..theme::CLEAR
        }
        .css()
    )
}

pub fn css(p: &Palette) -> String {
    format!("{}\n{}", cards_css(p), super::dictionary::css(p))
}

fn cards_css(p: &Palette) -> String {
    let clear = theme::CLEAR.css();
    let side = theme::BUTTON_SIDE - theme::BUTTON_OUTLINE;
    format!(
        "window.squiggle {{ background: transparent; box-shadow: none; }}
         .squiggle-card {{ font-family: \"{family}\", sans-serif; background-color: {surface}; border: {edge}px solid {edge_color}; border-radius: {corner}px; box-shadow: {tight}, {soft}; }}
         .squiggle-dot {{ border-radius: {dot_radius}px; }}
         .squiggle-dot.squiggle-mistake {{ background-color: {mistake}; }}
         .squiggle-dot.squiggle-advice {{ background-color: {advice}; }}
         .squiggle-title {{ color: {text2}; {title} }}
         .squiggle-sentence {{ color: {text}; {sentence} line-height: {line_height}; }}
         button.squiggle-button {{ all: unset; border: {outline}px solid {clear}; border-radius: {control}px; padding: 0 {side}px; transition: background-color {hover}ms, border-color {hover}ms; }}
         button.squiggle-button label {{ color: {text}; {button} }}
         button.squiggle-main {{ background-color: {ink}; }}
         button.squiggle-main:hover {{ background-color: {ink_hover}; }}
         button.squiggle-main label {{ color: {on_ink}; {button_main} }}
         button.squiggle-plain {{ border-color: {line}; }}
         button.squiggle-plain:hover {{ background-color: {fill}; border-color: {clear}; }}
         button.squiggle-more, button.squiggle-item {{ all: unset; border-radius: {control}px; transition: background-color {hover}ms; }}
         button.squiggle-more:hover, button.squiggle-more.squiggle-open, button.squiggle-item:hover {{ background-color: {fill}; }}
         button.squiggle-item label {{ color: {text}; {button} }}
         .squiggle-menu {{ background-color: {surface}; border: {edge}px solid {edge_color}; border-radius: {corner}px; box-shadow: {tight}, {soft}; padding: {menu_pad}px; }}
         .squiggle-rule {{ background-color: {line}; }}",
        family = theme::FONT_FAMILY,
        surface = p.surface.css(),
        edge = edge_width(),
        edge_color = p.edge.css(),
        corner = theme::CORNER,
        tight = shadow(p, theme::SHADOW_TIGHT),
        soft = shadow(p, theme::SHADOW_SOFT),
        dot_radius = theme::DOT / 2.0,
        mistake = theme::MISTAKE.css(),
        advice = theme::ADVICE.css(),
        text = p.text.css(),
        text2 = p.text2.css(),
        title = font(theme::CARD_TITLE),
        sentence = font(theme::SENTENCE),
        line_height = theme::SENTENCE_LINE / theme::SENTENCE.size,
        outline = theme::BUTTON_OUTLINE,
        control = theme::CORNER_CONTROL,
        hover = theme::HOVER_FADE.as_millis(),
        button = font(theme::BUTTON),
        ink = p.ink.css(),
        ink_hover = p.ink_hover.css(),
        on_ink = p.on_ink.css(),
        button_main = font(theme::BUTTON_MAIN),
        line = p.line.css(),
        fill = p.fill.css(),
        menu_pad = theme::MENU_PAD as i32 - edge_width(),
    )
}

/// The sentence with its changes in place, as Pango markup, and where each
/// new word is in the text it lays out: what a fix takes out struck in red
/// (blue for clearer wording), what it puts there bold and green, padded so
/// its fill has room.
fn sentence_markup(bits: &[Bit], kind: Kind, p: &Palette) -> (String, Vec<Range<usize>>) {
    let gone = theme::gone(p, kind).hex();
    let mut news = Vec::new();
    let mut laid_out = 0;
    let markup = bits
        .iter()
        .map(|bit| {
            let shown = match bit {
                Bit::Same(text) | Bit::Gone(text) => text.clone(),
                Bit::New(text) => format!("{0}{text}{0}", theme::NEW_WORD_PAD),
            };
            let escaped = gtk::glib::markup_escape_text(&shown);
            let start = laid_out;
            laid_out += shown.len();
            match bit {
                Bit::Same(_) => escaped.to_string(),
                Bit::Gone(_) => {
                    format!(r#"<span foreground="{gone}" strikethrough="true">{escaped}</span>"#)
                }
                Bit::New(_) => {
                    news.push(start..laid_out);
                    format!(
                        r#"<span foreground="{}" weight="{}">{escaped}</span>"#,
                        p.new.hex(),
                        theme::NEW_WORDS.weight.css()
                    )
                }
            }
        })
        .collect();
    (markup, news)
}

/// The sentence font's own height, which the fill behind a new word is
/// sized from.
fn natural_height(label: &gtk::Label) -> f64 {
    let context = label.pango_context();
    let mut font = gtk::pango::FontDescription::new();
    font.set_family(theme::FONT_FAMILY);
    font.set_absolute_size(theme::SENTENCE.size * f64::from(gtk::pango::SCALE));
    let metrics = context.metrics(Some(&font), None);
    f64::from(metrics.ascent() + metrics.descent()) / f64::from(gtk::pango::SCALE)
}

/// Where a run of the laid-out text is drawn, one rectangle per line it
/// crosses, in the label's own coordinates.
fn line_rects(layout: &gtk::pango::Layout, range: Range<usize>) -> Vec<Rect> {
    let text = layout.text();
    let scale = f64::from(gtk::pango::SCALE);
    let mut lines: Vec<Rect> = Vec::new();
    for (offset, _) in text[range.clone()].char_indices() {
        let at = layout.index_to_pos((range.start + offset) as i32);
        let char_rect = Rect {
            x: f64::from(at.x()) / scale,
            y: f64::from(at.y()) / scale,
            w: f64::from(at.width()) / scale,
            h: f64::from(at.height()) / scale,
        };
        match lines.last_mut() {
            Some(line) if (line.y - char_rect.y).abs() < 1.0 => {
                let right = (char_rect.x + char_rect.w).max(line.x + line.w);
                line.x = line.x.min(char_rect.x);
                line.w = right - line.x;
            }
            _ => lines.push(char_rect),
        }
    }
    lines
}

/// The sentence over the fills of its new words. The label draws the words;
/// the fills are drawn in a sibling beneath it in the same cell, rounded as
/// Pango's own background cannot be, from where the label's layout put them.
fn sentence_view(bits: &[Bit], kind: Kind, p: &'static Palette) -> gtk::Grid {
    let (markup, news) = sentence_markup(bits, kind, p);
    let body = gtk::Label::new(None);
    body.set_markup(&markup);
    body.add_css_class("squiggle-sentence");
    body.set_xalign(0.0);
    body.set_valign(gtk::Align::Start);
    // Without it the grid's column is as narrow as that capped natural width,
    // and the sentence broke after every word.
    body.set_hexpand(true);
    body.set_wrap(true);
    // A wrapping label's natural width is its whole text on one line, and the
    // layer surface took that width; capped, the card's width decides.
    body.set_max_width_chars(1);
    body.set_lines(5);
    body.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let fills = gtk::DrawingArea::new();
    let words = body.clone();
    fills.set_draw_func(move |_, context, _, _| {
        let layout = words.layout();
        let (left, top) = words.layout_offsets();
        let height = theme::new_word_fill_height(natural_height(&words));
        for run in news.iter().cloned() {
            for line in line_rects(&layout, run) {
                let middle = f64::from(top) + line.y + line.h / 2.0;
                fill(
                    context,
                    Rect {
                        x: f64::from(left) + line.x,
                        y: middle - height / 2.0,
                        w: line.w,
                        h: height,
                    },
                    theme::CORNER_INLINE,
                    p.new_fill,
                );
            }
        }
    });

    let grid = gtk::Grid::new();
    grid.attach(&fills, 0, 0, 1, 1);
    grid.attach(&body, 0, 0, 1, 1);
    grid
}

fn kind_class(kind: Kind) -> &'static str {
    match kind {
        Kind::Spelling | Kind::Correctness => "squiggle-mistake",
        Kind::Clarity => "squiggle-advice",
    }
}

/// The dot in the underline's colour, then the title.
fn heading(card: &Card) -> gtk::Box {
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("squiggle-dot");
    dot.add_css_class(kind_class(card.kind));
    dot.set_size_request(theme::DOT as i32, theme::DOT as i32);
    dot.set_valign(gtk::Align::Center);

    let title = gtk::Label::new(Some(&card.title));
    title.add_css_class("squiggle-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_margin_start((theme::TITLE_INDENT - theme::DOT) as i32);
    // Room for the ⋯, laid over the title's end.
    title.set_margin_end((theme::MORE - theme::MORE_OUT + theme::BETWEEN_BUTTONS) as i32);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_height_request(theme::TITLE_LINE as i32);
    row.append(&dot);
    row.append(&title);
    row
}

/// Which buttons share a line, left to right, wrapping in the rare case they
/// do not fit `width`.
fn button_lines(widths: &[i32], width: i32, gap: i32) -> Vec<Vec<usize>> {
    let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
    let mut x = 0;
    for (index, &button) in widths.iter().enumerate() {
        if x > 0 && x + button > width {
            lines.push(Vec::new());
            x = 0;
        }
        x += button + gap;
        if let Some(line) = lines.last_mut() {
            line.push(index);
        }
    }
    lines
}

fn button(action: Action, primary: bool, index: usize, on_action: &OnAction) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_focusable(false);
    button.set_focus_on_click(false);
    button.set_height_request(theme::BUTTON_HEIGHT as i32);
    button.set_cursor_from_name(Some("pointer"));
    button.add_css_class("squiggle-button");
    button.add_css_class(if primary {
        "squiggle-main"
    } else {
        "squiggle-plain"
    });
    let on_action = Rc::clone(on_action);
    button.connect_clicked(move |_| on_action(action, index));
    let caption = gtk::Label::new(Some(action.label()));
    caption.set_valign(gtk::Align::Center);
    button.set_child(Some(&caption));
    button
}

/// An icon from `theme::icon`, `size` square, in `color`.
fn glyph(action: Action, size: f64, color: Rgba) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(size as i32);
    area.set_content_height(size as i32);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    area.set_can_target(false);
    area.set_draw_func(move |_, context, _, _| {
        let s = size / 16.0;
        context.set_source_rgba(color.r, color.g, color.b, color.a);
        context.set_line_width(theme::ICON_STROKE * s);
        context.set_line_cap(gtk::cairo::LineCap::Round);
        for shape in theme::icon(action) {
            context.new_path();
            let filled = match *shape {
                Shape::Dot { x, y, r } => {
                    context.arc(x * s, y * s, r * s, 0.0, std::f64::consts::TAU);
                    true
                }
                Shape::Ring { x, y, r } => {
                    context.arc(x * s, y * s, r * s, 0.0, std::f64::consts::TAU);
                    false
                }
                Shape::Line { from, to } => {
                    context.move_to(from.0 * s, from.1 * s);
                    context.line_to(to.0 * s, to.1 * s);
                    false
                }
                Shape::Bar { x, y, w, h, r } => {
                    let bar = Rect {
                        x: x * s,
                        y: y * s,
                        w: w * s,
                        h: h * s,
                    };
                    rounded(context, bar, r * s);
                    false
                }
            };
            // A failed stroke draws nothing, and there is nothing to do.
            let _ = if filled {
                context.fill()
            } else {
                context.stroke()
            };
        }
    });
    area
}

fn plain(button: &gtk::Button, class: &str, action: Action, index: usize, on_action: &OnAction) {
    button.set_focusable(false);
    button.set_focus_on_click(false);
    button.set_cursor_from_name(Some("pointer"));
    button.add_css_class(class);
    let on_action = Rc::clone(on_action);
    button.connect_clicked(move |_| on_action(action, index));
}

/// The ⋯ at the title's end, laid over the card.
fn more(index: usize, on_action: &OnAction, p: &Palette) -> gtk::Button {
    let button = gtk::Button::new();
    plain(&button, "squiggle-more", Action::More, index, on_action);
    button.set_size_request(theme::MORE as i32, theme::MORE as i32);
    button.set_halign(gtk::Align::End);
    button.set_valign(gtk::Align::Start);
    // From inside the card's hairline, as the column's margins are.
    let inset = edge_width();
    button.set_margin_end((theme::CARD_PAD - theme::MORE_OUT) as i32 - inset);
    button
        .set_margin_top((theme::CARD_PAD + (theme::TITLE_LINE - theme::MORE) / 2.0) as i32 - inset);
    button.set_child(Some(&glyph(Action::More, theme::MORE_ICON, p.text2)));
    button
}

/// The ⋯'s menu, where `theme::menu_at` puts it, hidden.
fn menu(index: usize, on_action: &OnAction, p: &Palette) -> (gtk::Box, Vec<(Action, gtk::Widget)>) {
    let at = theme::menu_at(Rect {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    });
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    menu.add_css_class("squiggle-menu");
    menu.set_visible(false);
    menu.set_halign(gtk::Align::End);
    menu.set_valign(gtk::Align::Start);
    let inset = edge_width();
    menu.set_margin_end(theme::MENU_INSET as i32 - inset);
    menu.set_margin_top(at.y as i32 - inset);
    menu.set_size_request(at.w as i32, at.h as i32);
    let mut rows = Vec::new();
    for (i, action) in MENU.into_iter().enumerate() {
        if i > 0 {
            let rule = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            rule.add_css_class("squiggle-rule");
            rule.set_height_request(1);
            let space = ((theme::MENU_RULE - 1.0) / 2.0) as i32;
            rule.set_margin_top(space);
            rule.set_margin_bottom(space);
            rule.set_margin_start(6);
            rule.set_margin_end(6);
            menu.append(&rule);
        }
        let item = gtk::Button::new();
        plain(&item, "squiggle-item", action, index, on_action);
        item.set_height_request(theme::BUTTON_HEIGHT as i32);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.set_margin_start((theme::BUTTON_SIDE - 2.0) as i32);
        row.append(&glyph(action, theme::MENU_ICON, p.text2));
        let caption = gtk::Label::new(Some(action.label()));
        caption.set_xalign(0.0);
        caption.set_valign(gtk::Align::Center);
        row.append(&caption);
        item.set_child(Some(&row));
        menu.append(&item);
        rows.push((action, item.upcast()));
    }
    (menu, rows)
}

pub fn build(card: &Card, index: usize, on_action: &OnAction, p: &'static Palette) -> Built {
    let buttons: Vec<(Action, gtk::Button)> = card
        .actions
        .iter()
        .enumerate()
        .map(|(position, &action)| (action, button(action, position == 0, index, on_action)))
        .collect();
    let gap = theme::BETWEEN_BUTTONS as i32;
    let inner = (theme::CARD_WIDTH - theme::CARD_PAD * 2.0) as i32;
    let widths: Vec<i32> = buttons
        .iter()
        .map(|(_, button)| button.measure(gtk::Orientation::Horizontal, -1).1)
        .collect();
    let stack = gtk::Box::new(gtk::Orientation::Vertical, gap);
    stack.set_margin_top(theme::ABOVE_BUTTONS as i32);
    for line in button_lines(&widths, inner, gap) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, gap);
        line.iter().for_each(|&at| row.append(&buttons[at].1));
        stack.append(&row);
    }

    let sentence = sentence_view(&card.bits, card.kind, p);
    sentence.set_margin_top(theme::UNDER_TITLE as i32);
    // The hairline is inside the card's size, as on macOS.
    let pad = theme::CARD_PAD as i32 - edge_width();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.set_margin_top(pad);
    column.set_margin_bottom(pad);
    column.set_margin_start(pad);
    column.set_margin_end(pad);
    column.append(&heading(card));
    column.append(&sentence);
    column.append(&stack);

    let more = more(index, on_action, p);
    let (menu, menu_rows) = menu(index, on_action, p);
    // The ⋯ and its menu lie over the card's words; neither changes its
    // size.
    let layers = gtk::Overlay::new();
    layers.set_child(Some(&column));
    layers.add_overlay(&more);
    layers.add_overlay(&menu);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("squiggle-card");
    root.append(&layers);
    // The shadow is the card's own, and falls in this margin of the surface.
    let (left, right, top, bottom) = theme::SHADE;
    root.set_margin_start(left as i32);
    root.set_margin_end(right as i32);
    root.set_margin_top(top as i32);
    root.set_margin_bottom(bottom as i32);
    let mut rows = vec![(Action::More, more.clone().upcast())];
    rows.extend(
        buttons
            .into_iter()
            .map(|(action, button)| (action, button.upcast())),
    );
    rows.extend(menu_rows);
    Built {
        root,
        rows,
        more,
        menu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits() -> [Bit; 3] {
        [
            Bit::Same("I ".into()),
            Bit::Gone("goed".into()),
            Bit::New("went".into()),
        ]
    }

    #[test]
    fn the_sentence_marks_what_goes_and_what_comes() {
        let (markup, _) = sentence_markup(&bits(), Kind::Correctness, &theme::LIGHT);
        assert_eq!(
            markup,
            format!(
                r##"I <span foreground="{}" strikethrough="true">goed</span><span foreground="{}" weight="600">{pad}went{pad}</span>"##,
                theme::LIGHT.gone.hex(),
                theme::LIGHT.new.hex(),
                pad = theme::NEW_WORD_PAD
            )
        );
    }

    #[test]
    fn a_new_word_is_found_in_the_text_with_its_padding() {
        let (_, news) = sentence_markup(&bits(), Kind::Correctness, &theme::LIGHT);
        let text = format!("I goed{0}went{0}", theme::NEW_WORD_PAD);
        assert_eq!(news.len(), 1);
        assert_eq!(
            &text[news[0].clone()],
            format!("{0}went{0}", theme::NEW_WORD_PAD)
        );
    }

    #[test]
    fn clearer_wording_strikes_in_blue() {
        let bits = [Bit::Gone("a".into())];
        let (markup, _) = sentence_markup(&bits, Kind::Clarity, &theme::DARK);
        assert!(markup.contains(&theme::DARK.gone_advice.hex()));
    }

    #[test]
    fn markup_characters_in_the_text_are_escaped() {
        let (markup, _) = sentence_markup(
            &[Bit::Same("a < b & c".into())],
            Kind::Correctness,
            &theme::LIGHT,
        );
        assert_eq!(markup, "a &lt; b &amp; c");
    }

    #[test]
    fn buttons_that_do_not_fit_wrap() {
        assert_eq!(button_lines(&[100, 100], 308, 8), [[0, 1]]);
        assert_eq!(button_lines(&[200, 120], 308, 8), [[0], [1]]);
        assert_eq!(button_lines(&[400], 308, 8), [[0]]);
    }

    #[test]
    fn the_stylesheet_is_the_palette_and_the_measures() {
        for p in [&theme::LIGHT, &theme::DARK] {
            let css = css(p);
            for color in [p.surface, p.edge, p.ink, p.ink_hover, p.fill, p.line] {
                assert!(css.contains(&color.css()));
            }
            assert!(css.contains("border-radius: 8px"));
            assert!(css.contains("border-radius: 6px"));
            assert!(css.contains("0 14px 18px -12px"));
            assert!(css.contains("0 1px 2px 0px"));
            assert!(css.contains("font-size: 13.5px; font-weight: 600"));
            assert!(css.contains("line-height: 1.6"));
        }
    }
}
