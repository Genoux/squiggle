//! What Gramly draws on top of other apps: underlines and the suggestion
//! card, as two wlr-layer-shell surfaces.
//!
//! The underlines' surface is mapped once and never unmapped: a layer
//! surface that is mapped again goes on top of the others in its layer, and
//! the card must stay above the underlines. Clearing the drawing hides them.
//! It covers only the field's area, as macOS sizes its panel, and shrinks to
//! a pixel when there is nothing to draw: a surface over the whole output
//! kept fullscreen video and games from direct scanout. Its input region is
//! empty, so clicks land on the text beneath. The card
//! is the opposite: unmapped to hide, mapped to show, which puts it on top,
//! and it never takes keyboard focus, so accepting a fix leaves the caret in
//! the field.
//!
//! Layer surfaces are placed from their output's corner, so every global
//! rectangle crosses to the output's own coordinates once, here.

use super::card::{self, Built, OnAction};
use super::hypr::{self, Monitor};
use crate::surface::{self, Action, Card, Mark, Point, Rect, place_card, theme};
use gtk::prelude::*;
use gtk::{cairo, gdk};
use gtk4 as gtk;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Outputs rarely change; asking Hyprland on every tick would be a socket
/// round trip for nothing.
const LAYOUT_FRESH: Duration = Duration::from_secs(2);

/// The output a rectangle is on, by its middle; the first when it is on none.
fn monitor_around(monitors: &[Monitor], rect: Rect) -> Option<&Monitor> {
    let middle = Point {
        x: rect.x + rect.w / 2.0,
        y: rect.y + rect.h / 2.0,
    };
    monitors
        .iter()
        .find(|monitor| monitor.frame.contains(middle))
        .or_else(|| monitors.first())
}

/// A global point as margins from an output's top-left corner.
fn margins(at: Point, frame: Rect) -> (i32, i32) {
    (
        (at.x - frame.x).round() as i32,
        (at.y - frame.y).round() as i32,
    )
}

/// Where the underline surface goes for a field's `area`, as margins from
/// the output's corner and a size in whole pixels, rounded out.
fn cover(area: Rect, frame: Rect) -> ((i32, i32), (i32, i32)) {
    let origin = Point {
        x: area.x,
        y: area.y,
    };
    (
        margins(origin, frame),
        (area.w.ceil().max(1.0) as i32, area.h.ceil().max(1.0) as i32),
    )
}

/// What the surface shrinks to with nothing to draw.
const IDLE: ((i32, i32), (i32, i32)) = ((0, 0), (1, 1));

fn relative(mark: &Mark, origin: Rect) -> Mark {
    Mark {
        rect: Rect {
            x: mark.rect.x - origin.x,
            y: mark.rect.y - origin.y,
            ..mark.rect
        },
        ..mark.clone()
    }
}

fn gdk_monitor(connector: &str) -> Option<gdk::Monitor> {
    gdk::Display::default()?
        .monitors()
        .iter::<gdk::Monitor>()
        .flatten()
        .find(|monitor| monitor.connector().as_deref() == Some(connector))
}

/// A layer surface on one output.
struct Surface {
    window: gtk::Window,
    output: Option<String>,
}

impl Surface {
    fn new(namespace: &str, edges: &[Edge]) -> Self {
        let window = gtk::Window::new();
        window.set_decorated(false);
        window.add_css_class("gramly");
        window.init_layer_shell();
        window.set_namespace(Some(namespace));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::None);
        // Ignores other surfaces' exclusive zones, so margins count from the
        // output's own corner and match Hyprland's global coordinates.
        window.set_exclusive_zone(-1);
        for &edge in edges {
            window.set_anchor(edge, true);
        }
        Self {
            window,
            output: None,
        }
    }

    /// Moves the surface to `monitor`; whether it was not there already.
    fn on(&mut self, monitor: &Monitor) -> bool {
        let moved = self.output.as_deref() != Some(&monitor.name);
        if moved {
            self.window.set_monitor(gdk_monitor(&monitor.name).as_ref());
            self.output = Some(monitor.name.clone());
        }
        moved
    }
}

fn click_through(window: &gtk::Window) {
    if let Some(surface) = window.surface() {
        surface.set_input_region(Some(&cairo::Region::create()));
    }
}

fn rounded(context: &cairo::Context, rect: Rect, radius: f64) {
    let (right, bottom) = (rect.x + rect.w, rect.y + rect.h);
    let quarter = std::f64::consts::FRAC_PI_2;
    context.new_sub_path();
    context.arc(right - radius, rect.y + radius, radius, -quarter, 0.0);
    context.arc(right - radius, bottom - radius, radius, 0.0, quarter);
    context.arc(
        rect.x + radius,
        bottom - radius,
        radius,
        quarter,
        2.0 * quarter,
    );
    context.arc(
        rect.x + radius,
        rect.y + radius,
        radius,
        2.0 * quarter,
        3.0 * quarter,
    );
    context.close_path();
}

fn fill(context: &cairo::Context, rect: Rect, radius: f64, color: theme::Rgba) {
    rounded(context, rect, radius);
    context.set_source_rgba(color.r, color.g, color.b, color.a);
    // A failed fill leaves the context in an error state that every later
    // call reports again; there is nothing to do about it mid-frame.
    let _ = context.fill();
}

/// A 2px line straddling the word's bottom edge, a fill behind it when
/// highlighted, and for a word just accepted the fill alone, in green.
fn paint(context: &cairo::Context, marks: &[Mark]) {
    for mark in marks {
        let Rect { x, y, w, h } = mark.rect;
        let color = if mark.flash {
            theme::ACCEPTED
        } else {
            theme::kind(mark.kind)
        };
        if mark.highlighted {
            let tint = if mark.flash {
                color
            } else {
                theme::Rgba { a: 0.2, ..color }
            };
            fill(
                context,
                Rect {
                    x: x - 1.0,
                    y,
                    w: w + 2.0,
                    h,
                },
                3.0,
                tint,
            );
        }
        if !mark.flash {
            fill(
                context,
                Rect {
                    x,
                    y: y + h - 1.0,
                    w,
                    h: 2.0,
                },
                1.0,
                color,
            );
        }
    }
}

/// The card on screen: what was built and where it went.
struct Shown {
    built: Built,
    frame: Rect,
}

pub struct LayerOverlay {
    on_action: OnAction,
    layout: Option<(Instant, Vec<Monitor>)>,
    underlines: Surface,
    canvas: gtk::DrawingArea,
    /// Marks in the underline surface's own coordinates, as painted.
    painted: Rc<RefCell<Vec<Mark>>>,
    /// The marks last given, to redraw only when they change, and the area
    /// the surface was sized to for them.
    drawn: Vec<Mark>,
    covering: Option<Rect>,
    card: Surface,
    shown: Option<Shown>,
}

impl LayerOverlay {
    /// `on_action` is what a click on a row calls. The card is a real surface
    /// that gets pointer events, so rows light and click on their own.
    pub fn new(on_action: OnAction) -> Self {
        let provider = gtk::CssProvider::new();
        provider.load_from_data(&card::css());
        // Above the user's own gtk.css: a full theme there (matugen's, 300KB)
        // painted every window grey, the whole-screen underline layer
        // included, and restyled the card's buttons.
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
            );
        }
        let underlines = Surface::new("gramly-marks", &[Edge::Left, Edge::Top]);
        let painted = Rc::new(RefCell::new(Vec::new()));
        let canvas = gtk::DrawingArea::new();
        let shared = Rc::clone(&painted);
        canvas.set_draw_func(move |_, context, _, _| paint(context, &shared.borrow()));
        underlines.window.set_child(Some(&canvas));
        WidgetExt::realize(&underlines.window);
        click_through(&underlines.window);
        underlines.window.connect_map(click_through);
        Self {
            on_action,
            layout: None,
            underlines,
            canvas,
            painted,
            drawn: Vec::new(),
            covering: None,
            card: Surface::new("gramly-card", &[Edge::Left, Edge::Top]),
            shown: None,
        }
    }

    fn cover(&mut self, (margin, size): ((i32, i32), (i32, i32))) {
        let window = &self.underlines.window;
        window.set_margin(Edge::Left, margin.0);
        window.set_margin(Edge::Top, margin.1);
        self.canvas.set_content_width(size.0);
        self.canvas.set_content_height(size.1);
        window.set_default_size(size.0, size.1);
    }

    /// Puts the card back on top of the underlines: a surface that is
    /// mapped again goes above the others, and moving to another output may
    /// map the underlines again.
    fn raise_card(&self) {
        if self.shown.is_some() {
            self.card.window.set_visible(false);
            self.card.window.set_visible(true);
        }
    }

    fn monitor_at(&mut self, rect: Rect) -> Option<Monitor> {
        let stale = self
            .layout
            .as_ref()
            .is_none_or(|(read, monitors)| monitors.is_empty() || read.elapsed() >= LAYOUT_FRESH);
        if stale {
            self.layout = Some((Instant::now(), hypr::monitors()));
        }
        let (_, monitors) = self.layout.as_ref()?;
        monitor_around(monitors, rect).cloned()
    }
}

impl surface::Overlay for LayerOverlay {
    fn marks(&mut self, area: Rect, marks: Vec<Mark>) {
        if marks.is_empty() {
            self.hide_marks();
            return;
        }
        let Some(monitor) = self.monitor_at(area) else {
            return;
        };
        let moved = self.underlines.output.as_deref() != Some(&monitor.name);
        if self.drawn == marks && !moved && self.covering == Some(area) {
            return;
        }
        if self.underlines.on(&monitor) {
            self.raise_card();
        }
        self.cover(cover(area, monitor.frame));
        self.covering = Some(area);
        *self.painted.borrow_mut() = marks.iter().map(|mark| relative(mark, area)).collect();
        self.drawn = marks;
        self.canvas.queue_draw();
        if !self.underlines.window.is_visible() {
            self.underlines.window.set_visible(true);
        }
    }

    fn hide_marks(&mut self) {
        if self.covering.take().is_some() {
            self.drawn.clear();
            self.painted.borrow_mut().clear();
            self.cover(IDLE);
            self.canvas.queue_draw();
        }
    }

    fn show_card(&mut self, index: usize, card: &Card, word: Rect) {
        let Some(monitor) = self.monitor_at(word) else {
            return;
        };
        let built = card::build(card, index, &self.on_action);
        let (width, height) = built.size;
        let origin = place_card(word, (width, height), monitor.usable);
        let (left, top) = margins(origin, monitor.frame);
        self.card.on(&monitor);
        self.card.window.set_margin(Edge::Left, left);
        self.card.window.set_margin(Edge::Top, top);
        self.card.window.set_child(Some(&built.root));
        self.card.window.set_visible(true);
        self.shown = Some(Shown {
            built,
            frame: Rect {
                x: origin.x,
                y: origin.y,
                w: width,
                h: height,
            },
        });
    }

    fn hide_card(&mut self) {
        if self.shown.take().is_some() {
            self.card.window.set_visible(false);
        }
    }

    fn card_frame(&self) -> Option<Rect> {
        self.shown.as_ref().map(|shown| shown.frame)
    }

    fn card_rows(&self) -> Vec<(Action, Rect)> {
        let Some(shown) = &self.shown else {
            return Vec::new();
        };
        shown
            .built
            .rows
            .iter()
            .filter_map(|(action, row)| {
                let bounds = row.compute_bounds(&shown.built.root)?;
                Some((
                    *action,
                    Rect {
                        x: shown.frame.x + f64::from(bounds.x()),
                        y: shown.frame.y + f64::from(bounds.y()),
                        w: f64::from(bounds.width()),
                        h: f64::from(bounds.height()),
                    },
                ))
            })
            .collect()
    }

    fn hover(&mut self, _pointer: Point) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use gramly_core::Kind;

    fn monitor(name: &str, x: f64) -> Monitor {
        let frame = Rect {
            x,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        Monitor {
            name: name.into(),
            frame,
            usable: frame,
        }
    }

    fn word_at(x: f64) -> Rect {
        Rect {
            x,
            y: 100.0,
            w: 50.0,
            h: 20.0,
        }
    }

    #[test]
    fn a_word_belongs_to_the_output_its_middle_is_on() {
        let outputs = [monitor("DP-1", 0.0), monitor("DP-2", 1920.0)];
        assert_eq!(
            monitor_around(&outputs, word_at(2000.0)).unwrap().name,
            "DP-2"
        );
        assert_eq!(
            monitor_around(&outputs, word_at(100.0)).unwrap().name,
            "DP-1"
        );
    }

    #[test]
    fn a_word_on_no_output_takes_the_first() {
        let outputs = [monitor("DP-1", 0.0), monitor("DP-2", 1920.0)];
        assert_eq!(
            monitor_around(&outputs, word_at(-500.0)).unwrap().name,
            "DP-1"
        );
        assert_eq!(monitor_around(&[], word_at(0.0)), None);
    }

    #[test]
    fn margins_count_from_the_outputs_corner() {
        let second = monitor("DP-2", 1920.0);
        assert_eq!(
            margins(
                Point {
                    x: 2000.4,
                    y: 130.6
                },
                second.frame
            ),
            (80, 131)
        );
    }

    #[test]
    fn a_mark_moves_with_the_surface_and_keeps_its_size() {
        let mark = Mark {
            rect: word_at(2000.0),
            kind: Kind::Spelling,
            highlighted: true,
            flash: false,
        };
        let area = Rect {
            x: 1990.0,
            y: 90.0,
            w: 300.0,
            h: 40.0,
        };
        let local = relative(&mark, area);
        assert_eq!(
            local.rect,
            Rect {
                x: 10.0,
                y: 10.0,
                w: 50.0,
                h: 20.0
            }
        );
        assert!(local.highlighted);
    }

    #[test]
    fn the_surface_covers_the_area_from_the_outputs_corner() {
        let area = Rect {
            x: 2000.4,
            y: 130.6,
            w: 300.2,
            h: 40.0,
        };
        assert_eq!(
            cover(area, monitor("DP-2", 1920.0).frame),
            ((80, 131), (301, 40))
        );
    }

    #[test]
    fn the_surface_is_never_smaller_than_a_pixel() {
        let area = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        assert_eq!(cover(area, monitor("DP-1", 0.0).frame).1, (1, 1));
    }
}
