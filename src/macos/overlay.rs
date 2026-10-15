//! What Gramly draws on top of other apps: underlines and the suggestion
//! card. There is no Accept-all pill: Grammarly has none, and a badge in
//! the corner of every field was noise.
//!
//! All three are non-activating panels. Clicking Accept must not take focus
//! away from the field being corrected - an activating window would, the
//! caret would leave the text, and the user would have to click back in to
//! keep typing. The underline panel and the card's shadow ignore the mouse
//! entirely, so clicks land on the text beneath them exactly as if they were
//! not there.
//!
//! Layer changes sit inside a transaction with actions disabled: Core
//! Animation gives every new layer an implicit position animation, which
//! drew each underline sliding in from the panel's corner. The one motion is
//! deliberate: the card fades in (`fade_in`), with an explicit animation.

use crate::macos::theme::{self, Palette, Rgba, Weight};
use gramly_core::diff::{Piece, pieces};
use gramly_core::{Fix, Kind, Source};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSBaselineOffsetAttributeName, NSButton, NSColor, NSFontAttributeName,
    NSForegroundColorAttributeName, NSLayoutManager, NSLineBreakMode, NSMutableParagraphStyle,
    NSPanel, NSParagraphStyleAttributeName, NSScreen, NSStatusWindowLevel,
    NSStrikethroughColorAttributeName, NSStrikethroughStyleAttributeName, NSTextContainer,
    NSTextField, NSTextStorage, NSTextView, NSView, NSWindowAnimationBehavior,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGPath;
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableAttributedString, NSNotFound,
    NSNumber, NSRange, NSString,
};
use objc2_quartz_core::{
    CABasicAnimation, CALayer, CAMediaTiming, CAMediaTimingFunction, CATransaction,
    kCAMediaTimingFunctionEaseOut,
};
use std::cell::RefCell;
use std::ptr::NonNull;
use std::rc::Rc;

/// One underline, already in Cocoa screen coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub rect: CGRect,
    pub kind: Kind,
    pub highlighted: bool,
    /// Just accepted: a green glow and no underline.
    pub flash: bool,
}

/// Accessibility reports rectangles from the top-left of the primary display
/// with y growing down; Cocoa places windows from its bottom-left with y
/// growing up. Every rectangle crosses this once, here.
pub fn to_cocoa(rect: CGRect, mtm: MainThreadMarker) -> CGRect {
    let screens = NSScreen::screens(mtm);
    let height = screens
        .firstObject()
        .map(|primary| primary.frame().size.height)
        .unwrap_or(0.0);
    CGRect::new(
        CGPoint::new(rect.origin.x, height - rect.origin.y - rect.size.height),
        rect.size,
    )
}

/// The usable part (no menu bar, no Dock) of the screen a rectangle is on,
/// in Cocoa coordinates; the main screen's when it is on none.
fn screen_around(rect: CGRect, mtm: MainThreadMarker) -> CGRect {
    let middle = CGPoint::new(
        rect.origin.x + rect.size.width / 2.0,
        rect.origin.y + rect.size.height / 2.0,
    );
    let screens = NSScreen::screens(mtm);
    screens
        .iter()
        .find(|screen| contains(screen.frame(), middle))
        .or_else(|| NSScreen::mainScreen(mtm))
        .map(|screen| screen.visibleFrame())
        .unwrap_or(rect)
}

pub fn contains(rect: CGRect, point: CGPoint) -> bool {
    point.x >= rect.origin.x
        && point.x <= rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y <= rect.origin.y + rect.size.height
}

fn still<T>(change: impl FnOnce() -> T) -> T {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    let result = change();
    CATransaction::commit();
    result
}

fn panel(mtm: MainThreadMarker, interactive: bool, level: isize) -> Retained<NSPanel> {
    let frame = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(10.0, 10.0));
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    // Gramly draws the card's shadow itself (`shade`), so it is the same on
    // Linux; the system's would sit under it.
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(!interactive);
    // Not `setFloatingPanel`: it resets the level to NSFloatingWindowLevel,
    // which put every panel on one level and drew underlines over the card
    // whenever they redrew last.
    panel.setLevel(level);
    panel.setAnimationBehavior(NSWindowAnimationBehavior::None);
    // An accessory app is never active, and a panel that hides on
    // deactivation would therefore never show at all.
    panel.setHidesOnDeactivate(false);
    panel.setBecomesKeyOnlyIfNeeded(true);
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
    view.setWantsLayer(true);
    panel.setContentView(Some(&view));
    panel
}

fn layer_view(frame: CGRect, mtm: MainThreadMarker) -> Retained<NSView> {
    let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
    view.setWantsLayer(true);
    view
}

fn with_alpha(c: Rgba, a: f64) -> Rgba {
    Rgba { a, ..c }
}

pub struct Overlay {
    mtm: MainThreadMarker,
    marks_panel: Retained<NSPanel>,
    drawn: Vec<Mark>,
    card: Retained<NSPanel>,
    /// The card's shadow, in a panel of its own that ignores the mouse: in
    /// the card's panel, the shadow's pixels would take clicks meant for the
    /// text below the card.
    shade: Retained<NSPanel>,
    /// The fix the card is showing.
    pub card_for: Option<usize>,
    /// The card's buttons, and which one the pointer is over.
    rows: Vec<Row>,
    lit: Option<usize>,
}

impl Overlay {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            mtm,
            marks_panel: panel(mtm, false, NSStatusWindowLevel),
            drawn: Vec::new(),
            // One level above the underlines, so a line from the text under
            // the card never crosses it.
            card: panel(mtm, true, NSStatusWindowLevel + 1),
            shade: panel(mtm, false, NSStatusWindowLevel),
            card_for: None,
            rows: Vec::new(),
            lit: None,
        }
    }

    /// Redraws the underlines only when they changed: rebuilding layers on
    /// every tick would cost a compositor frame twenty times a second for
    /// nothing.
    pub fn marks(&mut self, area: CGRect, marks: Vec<Mark>) {
        if marks.is_empty() {
            self.hide_marks();
            return;
        }
        if self.drawn == marks && self.marks_panel.isVisible() && self.marks_panel.frame() == area {
            return;
        }
        if std::env::var_os("GRAMLY_TRACE").is_some() {
            note!("drawing {} underlines in {area:?}", marks.len());
        }
        let Some(root) = self.marks_panel.contentView().and_then(|v| v.layer()) else {
            return;
        };
        self.marks_panel.setFrame_display(area, false);
        still(|| {
            unsafe { root.setSublayers(None) };
            for mark in &marks {
                let x = mark.rect.origin.x - area.origin.x;
                let y = mark.rect.origin.y - area.origin.y;
                let color = theme::kind(mark.kind);
                let fill = if mark.flash {
                    Some(theme::ACCEPTED)
                } else if mark.highlighted {
                    Some(with_alpha(color, HOVER_FILL))
                } else {
                    None
                };
                if let Some(fill) = fill {
                    let layer = CALayer::new();
                    layer.setFrame(CGRect::new(
                        CGPoint::new(x - 1.0, y),
                        CGSize::new(mark.rect.size.width + 2.0, mark.rect.size.height),
                    ));
                    layer.setBackgroundColor(Some(&theme::cg(fill)));
                    layer.setCornerRadius(theme::CORNER_INLINE);
                    root.addSublayer(&layer);
                }
                if mark.flash {
                    continue;
                }
                let line = CALayer::new();
                line.setFrame(CGRect::new(
                    CGPoint::new(x, y - 1.0),
                    CGSize::new(mark.rect.size.width, 2.0),
                ));
                line.setBackgroundColor(Some(&theme::cg(color)));
                line.setCornerRadius(1.0);
                root.addSublayer(&line);
            }
        });
        self.drawn = marks;
        self.marks_panel.orderFrontRegardless();
    }

    pub fn hide_marks(&mut self) {
        if self.marks_panel.isVisible() {
            self.marks_panel.orderOut(None);
        }
        self.drawn.clear();
    }

    /// The card's buttons as (action, frame on screen in Cocoa coordinates),
    /// for `GRAMLY_STATE`.
    pub fn card_rows(&self) -> Vec<(String, CGRect)> {
        if !self.card.isVisible() {
            return Vec::new();
        }
        let origin = self.card.frame().origin;
        self.rows
            .iter()
            .map(|row| {
                let f = row.view.frame();
                (
                    row.action.name().to_string_lossy().into_owned(),
                    CGRect::new(
                        CGPoint::new(origin.x + f.origin.x, origin.y + f.origin.y),
                        f.size,
                    ),
                )
            })
            .collect()
    }

    pub fn card_frame(&self) -> Option<CGRect> {
        self.card.isVisible().then(|| self.card.frame())
    }

    /// The card for the hovered suggestion: its sentence with the change in
    /// place, then its buttons. One fix or several, a correction or clearer
    /// wording, the layout is the same; only the title and the buttons
    /// change.
    ///
    /// `group` is every fix in the hovered fix's sentence, by index, in order.
    pub fn show_card(
        &mut self,
        index: usize,
        group: &[(usize, Fix)],
        text: &str,
        word: CGRect,
        target: &AnyObject,
    ) {
        if self.card_for == Some(index) && self.card.isVisible() {
            return;
        }
        let palette = theme::palette(self.mtm);
        let Some(card) = card(index, group, text, target, palette, self.mtm) else {
            return;
        };
        self.rows = card.rows;
        self.lit = None;
        let size = card.view.frame().size;
        self.card.setContentView(Some(&card.view));

        // Close under the word: the less there is to cross, the less there is
        // to fall into. Shifted left so the card's text starts near the word.
        let gap = 6.0;
        let mut origin = CGPoint::new(word.origin.x - PAD, word.origin.y - gap - size.height);
        // Kept on the word's screen: a word near the right edge of a Slack
        // window put the sentence card half off it. Shifted sideways rather
        // than flipped, so it stays close to the word; above the word only
        // when there is no room below.
        const MARGIN: f64 = 8.0;
        let area = screen_around(word, self.mtm);
        let (low, high) = (area.origin.y, area.origin.y + area.size.height);
        let below = origin.y >= low + MARGIN;
        if !below {
            origin.y = (word.origin.y + word.size.height + gap).min(high - size.height - MARGIN);
        }
        let right = area.origin.x + area.size.width - size.width - MARGIN;
        origin.x = origin.x.min(right).max(area.origin.x + MARGIN);
        let frame = CGRect::new(origin, size);

        let shade = shade(size, palette, self.mtm);
        self.shade.setContentView(Some(&shade));
        self.shade.setFrame_display(
            CGRect::new(
                CGPoint::new(origin.x - SHADE.0, origin.y - SHADE.3),
                shade.frame().size,
            ),
            true,
        );
        self.card.setFrame_display(frame, true);
        // Settling towards its word: down onto a card below it, up onto
        // one above.
        let from = if below { RISE } else { -RISE };
        fade_in(&shade, from);
        fade_in(&card.view, from);
        self.shade.orderFrontRegardless();
        self.card.orderFrontRegardless();
        self.card_for = Some(index);
    }

    pub fn hide_card(&mut self) {
        if self.card.isVisible() {
            self.card.orderOut(None);
        }
        if self.shade.isVisible() {
            self.shade.orderOut(None);
        }
        self.card_for = None;
        self.rows.clear();
        self.lit = None;
    }

    /// Lights the button under the pointer. Driven by the same poll as the
    /// underlines: the card never becomes key, so it gets no tracking-area
    /// events of its own. The fade is short - long enough to read as a
    /// response, short enough that sweeping across the buttons does not
    /// trail.
    pub fn hover(&mut self, mouse: CGPoint) {
        if !self.card.isVisible() {
            return;
        }
        let origin = self.card.frame().origin;
        let at = CGPoint::new(mouse.x - origin.x, mouse.y - origin.y);
        let now = self
            .rows
            .iter()
            .position(|row| contains(row.view.frame(), at));
        if now == self.lit {
            return;
        }
        CATransaction::begin();
        CATransaction::setAnimationDuration(0.1);
        for (i, row) in self.rows.iter().enumerate() {
            row.light(Some(i) == now);
        }
        CATransaction::commit();
        self.lit = now;
    }

    pub fn hide_all(&mut self) {
        self.hide_marks();
        self.hide_card();
    }
}

/// The card's one motion: it fades in over `FADE` seconds, moving its last
/// `RISE` points towards its word.
const FADE: f64 = 0.14;
const RISE: f64 = 3.0;

/// The card's fade, moving from `from` (up positive). Explicit, so the
/// transactions that keep everything else still do not stop it.
fn fade_in(view: &NSView, from: f64) {
    let Some(layer) = view.layer() else {
        return;
    };
    let ease = CAMediaTimingFunction::functionWithName(unsafe { kCAMediaTimingFunctionEaseOut });
    for (path, from) in [("opacity", 0.0), ("transform.translation.y", from)] {
        let animation = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str(path)));
        unsafe {
            animation.setFromValue(Some(&NSNumber::new_f64(from)));
            animation.setToValue(Some(&NSNumber::new_f64(0.0)));
        }
        if path == "opacity" {
            unsafe { animation.setToValue(Some(&NSNumber::new_f64(1.0))) };
        }
        animation.setDuration(FADE);
        animation.setTimingFunction(Some(&ease));
        layer.addAnimation_forKey(&animation, Some(&NSString::from_str(path)));
    }
}

/// A button of the card: a container whose layer is the fill and outline,
/// its label, and a transparent button over the whole of it, so the button
/// is the target and not just its words.
struct Row {
    view: Retained<NSView>,
    rest: Rgba,
    hover: Rgba,
    /// The outline at rest; it gives way to the fill under the pointer.
    line: Option<Rgba>,
    /// What clicking it does ("accept:", "dismiss:"), for `GRAMLY_STATE`.
    action: Sel,
}

impl Row {
    fn light(&self, lit: bool) {
        let Some(layer) = self.view.layer() else {
            return;
        };
        let fill = if lit { self.hover } else { self.rest };
        layer.setBackgroundColor(Some(&theme::cg(fill)));
        if let Some(line) = self.line {
            layer.setBorderColor(Some(&theme::cg(if lit { theme::CLEAR } else { line })));
        }
    }
}

/// Every button: 32 high, 6 corners, 12 at the sides.
const BUTTON_H: f64 = 32.0;
const BUTTON_SIDE: f64 = 12.0;

fn label(
    text: &str,
    size: f64,
    weight: Weight,
    color: Rgba,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&theme::font(size, weight)));
    label.setTextColor(Some(&theme::ns(color)));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label
}

/// One button, its left edge at `x` and its bottom at `y`.
#[allow(clippy::too_many_arguments)]
fn button(
    words: &str,
    primary: bool,
    x: f64,
    y: f64,
    action: Sel,
    tag: isize,
    target: &AnyObject,
    p: &Palette,
    mtm: MainThreadMarker,
) -> Row {
    let caption = if primary {
        label(words, 13.5, Weight::Semibold, p.on_ink, mtm)
    } else {
        label(words, 13.5, Weight::Medium, p.text, mtm)
    };
    let fit = caption.fittingSize();
    let width = fit.width.ceil() + BUTTON_SIDE * 2.0;
    let view = layer_view(
        CGRect::new(CGPoint::new(x, y), CGSize::new(width, BUTTON_H)),
        mtm,
    );
    caption.setFrame(CGRect::new(
        CGPoint::new(BUTTON_SIDE, ((BUTTON_H - fit.height) / 2.0).round()),
        CGSize::new(fit.width.ceil(), fit.height),
    ));
    view.addSubview(&caption);
    let (rest, hover, line) = if primary {
        (p.ink, p.ink_hover, None)
    } else {
        (theme::CLEAR, p.fill, Some(p.line))
    };
    let row = Row {
        view,
        rest,
        hover,
        line,
        action,
    };
    still(|| {
        if let Some(layer) = row.view.layer() {
            layer.setCornerRadius(theme::CORNER_CONTROL);
            if line.is_some() {
                layer.setBorderWidth(1.0);
            }
        }
        row.light(false);
    });
    let cover = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(""),
            Some(target),
            Some(action),
            mtm,
        )
    };
    cover.setTag(tag);
    cover.setBordered(false);
    cover.setTransparent(true);
    cover.setFrame(CGRect::new(CGPoint::new(0.0, 0.0), row.view.frame().size));
    row.view.addSubview(&cover);
    row
}

/// A button in the card's style for other windows, at the origin. It does
/// not light under the pointer: only the card polls for that.
pub(crate) fn plain_button(
    words: &str,
    primary: bool,
    action: Sel,
    target: &AnyObject,
    p: &Palette,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    button(words, primary, 0.0, 0.0, action, 0, target, p, mtm).view
}

/// The title. The model's titles are written for this; Harper's messages are
/// written for a list ("Did you mean to spell recieve this way?") and read
/// oddly as a title, so local fixes get a plain one.
fn title(fix: &Fix, text: &str) -> String {
    let original = fix.original(text);
    // A doubled space's card: the change is invisible, the title says it.
    if !original.is_empty()
        && original.chars().all(char::is_whitespace)
        && fix.replacement.chars().all(char::is_whitespace)
    {
        return "Correct your spacing".into();
    }
    if fix.source == Source::Ai && !fix.message.is_empty() {
        return fix.message.clone();
    }
    match fix.kind {
        Kind::Spelling => "Correct your spelling",
        Kind::Correctness => "Correct your grammar",
        Kind::Clarity => gramly_core::improve::TITLE,
    }
    .to_string()
}

/// Space around the card's content.
const PAD: f64 = 16.0;
/// The card's width, the same for every suggestion.
const WIDTH: f64 = 340.0;
/// Between the title and the sentence, and between the sentence and the
/// buttons.
const UNDER_TITLE: f64 = 10.0;
const ABOVE_BUTTONS: f64 = 16.0;
const BETWEEN: f64 = 8.0;
/// The title's line.
const TITLE_H: f64 = 18.0;

pub(crate) struct Card {
    pub view: Retained<NSView>,
    rows: Vec<Row>,
}

/// Builds the card for the fix at `index`; None if it is not in `group`.
pub(crate) fn card(
    index: usize,
    group: &[(usize, Fix)],
    text: &str,
    target: &AnyObject,
    p: &Palette,
    mtm: MainThreadMarker,
) -> Option<Card> {
    let (_, fix) = group.iter().find(|(i, _)| *i == index)?;
    let tag = index as isize;
    let accept = ("Accept", sel!(accept:));
    let dismiss = ("Dismiss", sel!(dismiss:));
    // Clarity first: its card is never part of "Correct the sentence".
    let (heading, dot, members, actions): (String, Rgba, Vec<&Fix>, Vec<(&str, Sel)>) =
        if fix.kind == Kind::Clarity {
            (
                gramly_core::improve::TITLE.into(),
                theme::ADVICE,
                vec![fix],
                vec![accept, dismiss],
            )
        } else if group.len() > 1 {
            (
                "Correct the sentence".into(),
                theme::MISTAKE,
                group.iter().map(|(_, f)| f).collect(),
                vec![
                    ("Correct the sentence", sel!(acceptSentence:)),
                    ("Dismiss", sel!(dismissSentence:)),
                ],
            )
        } else {
            let mut actions = vec![accept, dismiss];
            if fix.kind == Kind::Spelling {
                actions.push(("Add to dictionary", sel!(learn:)));
            }
            (title(fix, text), theme::kind(fix.kind), vec![fix], actions)
        };

    let inner = WIDTH - PAD * 2.0;
    let gone = if fix.kind == Kind::Clarity {
        p.gone_advice
    } else {
        p.gone
    };
    let (sentence, sentence_h) = sentence_view(&snippet(text, &members), inner, gone, p, mtm);

    // Buttons left to right, wrapping in the rare case they do not fit.
    let mut placed: Vec<(Row, usize)> = Vec::new();
    let mut line = 0;
    let mut x = PAD;
    for (i, (words, action)) in actions.iter().enumerate() {
        let row = button(words, i == 0, 0.0, 0.0, *action, tag, target, p, mtm);
        let w = row.view.frame().size.width;
        if x > PAD && x + w > WIDTH - PAD {
            line += 1;
            x = PAD;
        }
        row.view.setFrameOrigin(CGPoint::new(x, 0.0));
        x += w + BETWEEN;
        placed.push((row, line));
    }
    let lines = (line + 1) as f64;
    let buttons_h = lines * BUTTON_H + (lines - 1.0) * BETWEEN;
    let height = PAD + TITLE_H + UNDER_TITLE + sentence_h + ABOVE_BUTTONS + buttons_h + PAD;

    let view = layer_view(
        CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(WIDTH, height)),
        mtm,
    );
    still(|| {
        if let Some(layer) = view.layer() {
            layer.setBackgroundColor(Some(&theme::cg(p.surface)));
            layer.setCornerRadius(theme::CORNER);
            // A hairline, not a border: half a point is one pixel on a
            // Retina screen.
            layer.setBorderWidth(0.5);
            layer.setBorderColor(Some(&theme::cg(p.edge)));
        }
    });

    let mut top = height - PAD;
    let marker = layer_view(
        CGRect::new(
            CGPoint::new(PAD, top - TITLE_H / 2.0 - 3.5),
            CGSize::new(7.0, 7.0),
        ),
        mtm,
    );
    still(|| {
        if let Some(layer) = marker.layer() {
            layer.setBackgroundColor(Some(&theme::cg(dot)));
            layer.setCornerRadius(3.5);
        }
    });
    view.addSubview(&marker);
    let heading = label(&heading, 13.0, Weight::Medium, p.text2, mtm);
    let heading_h = heading.fittingSize().height;
    heading.setFrame(CGRect::new(
        CGPoint::new(
            PAD + 15.0,
            top - TITLE_H + ((TITLE_H - heading_h) / 2.0).round(),
        ),
        CGSize::new(inner - 15.0, heading_h),
    ));
    view.addSubview(&heading);
    top -= TITLE_H + UNDER_TITLE + sentence_h;
    sentence.setFrameOrigin(CGPoint::new(PAD, top));
    view.addSubview(&sentence);

    let mut rows = Vec::new();
    for (row, line) in placed {
        let y = PAD + buttons_h - BUTTON_H - line as f64 * (BUTTON_H + BETWEEN);
        row.view
            .setFrameOrigin(CGPoint::new(row.view.frame().origin.x, y));
        view.addSubview(&row.view);
        rows.push(row);
    }
    Some(Card { view, rows })
}

/// How far the shadow reaches past the card: left, right, top, bottom.
pub(crate) const SHADE: (f64, f64, f64, f64) = (40.0, 40.0, 24.0, 64.0);

/// Behind a word whose card is open, its underline's colour at this alpha.
const HOVER_FILL: f64 = 0.16;

/// The card's two shadows as (inset, drop, blur, share of the palette's
/// strength): a tight one at its edge and a soft one below. The soft one
/// dropped 18 with a 22 blur, and the card floated like a bubble.
const SHADOW_TIGHT: (f64, f64, f64, f32) = (0.0, 1.0, 2.0, 0.4);
const SHADOW_SOFT: (f64, f64, f64, f32) = (12.0, 14.0, 18.0, 1.0);

/// The card's shadow, drawn from paths so nothing else is painted.
pub(crate) fn shade(size: CGSize, p: &Palette, mtm: MainThreadMarker) -> Retained<NSView> {
    let view = layer_view(
        CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(
                size.width + SHADE.0 + SHADE.1,
                size.height + SHADE.2 + SHADE.3,
            ),
        ),
        mtm,
    );
    let card = CGRect::new(CGPoint::new(SHADE.0, SHADE.3), size);
    still(|| {
        let Some(root) = view.layer() else {
            return;
        };
        for (inset, drop, blur, share) in [SHADOW_TIGHT, SHADOW_SOFT] {
            let strength = p.shadow * share;
            let layer = CALayer::new();
            layer.setFrame(view.bounds());
            let shape = CGRect::new(
                CGPoint::new(card.origin.x + inset, card.origin.y + inset),
                CGSize::new(
                    card.size.width - inset * 2.0,
                    card.size.height - inset * 2.0,
                ),
            );
            let path = unsafe {
                CGPath::with_rounded_rect(shape, theme::CORNER, theme::CORNER, std::ptr::null())
            };
            layer.setShadowPath(Some(&path));
            layer.setShadowColor(Some(&theme::cg(Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            })));
            layer.setShadowOpacity(strength);
            layer.setShadowRadius(blur);
            layer.setShadowOffset(CGSize::new(0.0, -drop));
            root.addSublayer(&layer);
        }
    });
    view
}

/// A piece of the card's sentence.
#[derive(Debug, Clone, PartialEq)]
enum Bit {
    Same(String),
    /// What goes, struck, in red (blue for clearer wording).
    Gone(String),
    /// What comes in, green on a green fill.
    New(String),
}

/// Words of context kept on each side of the change.
const CONTEXT: usize = 6;
/// A space that is the whole change, shown so the card is never blank.
const SPACE: &str = "\u{2423}";

/// The sentence the fixes are in, with each change in place: the struck
/// original and what replaces it. Cut to `CONTEXT` words either side, so a
/// long sentence does not make a long card.
///
/// It replaced a lone replacement word. Out of its sentence, "it's" did not
/// say why, and a removal had to be shown as what was left ("me the" for
/// "send me the the link").
fn snippet(text: &str, fixes: &[&Fix]) -> Vec<Bit> {
    let chars: Vec<char> = text.chars().collect();
    let mut fixes: Vec<&Fix> = fixes
        .iter()
        .copied()
        .filter(|f| f.start <= f.end && f.end <= chars.len())
        .collect();
    fixes.sort_by_key(|f| f.start);
    let Some(first) = fixes.first().map(|f| f.start) else {
        return Vec::new();
    };
    let last = fixes.iter().map(|f| f.end).max().unwrap_or(first);
    let (from, to) = gramly_core::sentence(text, first);
    let (from, to) = (from.min(first), to.max(last).min(chars.len()));

    let mut out = vec![Bit::Same(lead(&chars[from..first]))];
    let mut cursor = first;
    for fix in fixes {
        if fix.start < cursor {
            continue;
        }
        out.push(Bit::Same(chars[cursor..fix.start].iter().collect()));
        let original: String = chars[fix.start..fix.end].iter().collect();
        for piece in bridged(pieces(&original, &fix.replacement)) {
            match piece {
                Piece::Same(same) => out.push(Bit::Same(same)),
                Piece::Changed { was, now } => change(&mut out, &was, &now),
            }
        }
        cursor = fix.end;
    }
    out.push(Bit::Same(trail(&chars[cursor..to.max(cursor)])));

    // Merged and flattened: a sentence on a card is one line of thought.
    let mut tidy: Vec<Bit> = Vec::new();
    for bit in out {
        match (tidy.last_mut(), bit) {
            (_, Bit::Same(s)) if s.is_empty() => {}
            (Some(Bit::Same(prev)), Bit::Same(s)) => prev.push_str(&s.replace('\n', " ")),
            (_, Bit::Same(s)) => tidy.push(Bit::Same(s.replace('\n', " "))),
            (_, other) => tidy.push(other),
        }
    }
    tidy
}

/// Two changes with one word kept between them, as one change. A rewrite
/// shown word by word left "we" standing alone between two struck phrases
/// ("~~it would maybe be better if~~ we ~~could try to~~ should"), which
/// reads worse than the phrase struck and the new one beside it.
fn bridged(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for piece in pieces {
        if let Piece::Changed { was, now } = &piece
            && let [.., Piece::Changed { .. }, Piece::Same(kept)] = out.as_slice()
            && !kept.trim().is_empty()
            && !kept.trim().contains(char::is_whitespace)
        {
            let kept = kept.clone();
            out.pop();
            if let Some(Piece::Changed { was: w, now: n }) = out.last_mut() {
                w.push_str(&kept);
                w.push_str(was);
                n.push_str(&kept);
                n.push_str(now);
                continue;
            }
        }
        out.push(piece);
    }
    out
}

/// One changed stretch as bits. Spaces around a word stay plain text; a
/// change of spaces alone is shown with `SPACE`.
fn change(out: &mut Vec<Bit>, was: &str, now: &str) {
    let blank = |s: &str| s.trim().is_empty();
    if blank(was) && blank(now) {
        let (gone, came) = (was.chars().count(), now.chars().count());
        if gone > came {
            // Spaced either side, so the mark does not cling to a word.
            out.push(Bit::Same(now.into()));
            out.push(Bit::Gone(SPACE.repeat(gone - came)));
            out.push(Bit::Same(" ".into()));
        } else {
            out.push(Bit::Same(was.into()));
            out.push(Bit::New(SPACE.repeat(came - gone)));
        }
        return;
    }
    let edges = |s: &str| {
        let start = &s[..s.len() - s.trim_start().len()];
        let end = &s[s.trim_end().len()..];
        (start.to_string(), end.to_string())
    };
    // The spaces the sentence ends up with.
    let (lead, tail) = if blank(now) { edges(was) } else { edges(now) };
    out.push(Bit::Same(lead));
    if !blank(was) {
        out.push(Bit::Gone(was.trim().into()));
    }
    if !blank(now) {
        if !blank(was) {
            out.push(Bit::Same(" ".into()));
        }
        out.push(Bit::New(now.trim().into()));
    }
    out.push(Bit::Same(tail));
}

fn word_starts(chars: &[char]) -> Vec<usize> {
    (0..chars.len())
        .filter(|&i| !chars[i].is_whitespace() && (i == 0 || chars[i - 1].is_whitespace()))
        .collect()
}

/// The last `CONTEXT` words before the change.
fn lead(chars: &[char]) -> String {
    let starts = word_starts(chars);
    if starts.len() <= CONTEXT {
        return chars.iter().collect();
    }
    let cut = starts[starts.len() - CONTEXT];
    format!("…{}", chars[cut..].iter().collect::<String>())
}

/// The first `CONTEXT` words after the change.
fn trail(chars: &[char]) -> String {
    let starts = word_starts(chars);
    if starts.len() <= CONTEXT {
        return chars.iter().collect();
    }
    let cut = starts[CONTEXT];
    let kept: String = chars[..cut].iter().collect();
    format!("{}…", kept.trim_end())
}

/// The sentence's line height, and its type size.
const LINE: f64 = 24.0;
const TEXT: f64 = 15.0;
/// A no-break space either side of a new word, so its fill has room to
/// breathe without touching the letters beside it.
const PADDING: char = '\u{202F}';

/// The sentence, laid out `width` wide, over the fills of its new words.
/// Struck words take `gone`. Each word has its colour in one place: letters
/// and line in red, or green letters on their fill. Red letters under a
/// darker red line, or white letters under a red one, both read worse.
/// Returns the view and its height.
fn sentence_view(
    bits: &[Bit],
    width: f64,
    gone: Rgba,
    p: &Palette,
    mtm: MainThreadMarker,
) -> (Retained<NSView>, f64) {
    let regular = theme::font(TEXT, Weight::Regular);
    let bold = theme::font(TEXT, Weight::Semibold);
    // Centred in its 24pt line: fixed line heights put the extra room above
    // the letters.
    let natural = regular.ascender() - regular.descender();
    let raise = ((LINE - natural) / 2.0).max(0.0);
    let style = NSMutableParagraphStyle::new();
    style.setMinimumLineHeight(LINE);
    style.setMaximumLineHeight(LINE);

    let text = NSMutableAttributedString::new();
    let mut fills: Vec<NSRange> = Vec::new();
    for bit in bits {
        let (shown, font, color, struck) = match bit {
            Bit::Same(s) => (s.clone(), &regular, p.text, false),
            Bit::Gone(s) => (s.clone(), &regular, gone, true),
            Bit::New(s) => (format!("{PADDING}{s}{PADDING}"), &bold, p.new, false),
        };
        let start = text.length();
        text.appendAttributedString(&run(&shown, font, color, struck, raise, &style));
        if matches!(bit, Bit::New(_)) {
            fills.push(NSRange::new(start, text.length() - start));
        }
    }

    let storage = NSTextStorage::new();
    storage.setAttributedString(&text);
    let layout = NSLayoutManager::new();
    let container =
        NSTextContainer::initWithSize(NSTextContainer::alloc(), CGSize::new(width, f64::MAX));
    container.setLineFragmentPadding(0.0);
    layout.addTextContainer(&container);
    storage.addLayoutManager(&layout);
    layout.ensureLayoutForTextContainer(&container);
    let used = layout.usedRectForTextContainer(&container);
    let height = used.size.height.ceil().max(LINE);

    let frame = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height));
    let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
    let backdrop = layer_view(frame, mtm);
    // The text view is flipped, the backdrop is not: rectangles from the
    // layout are turned over to land under their words.
    let rects: Rc<RefCell<Vec<CGRect>>> = Rc::default();
    for range in fills {
        let glyphs = unsafe {
            layout.glyphRangeForCharacterRange_actualCharacterRange(range, std::ptr::null_mut())
        };
        let found = Rc::clone(&rects);
        let collect = block2::StackBlock::new(move |rect: CGRect, _stop: NonNull<Bool>| {
            found.borrow_mut().push(rect);
        });
        layout.enumerateEnclosingRectsForGlyphRange_withinSelectedGlyphRange_inTextContainer_usingBlock(
            glyphs,
            NSRange::new(NSNotFound as usize, 0),
            &container,
            &collect,
        );
    }
    let fill_h = (natural + 5.0).min(LINE - 1.0).round();
    still(|| {
        if let Some(root) = backdrop.layer() {
            for rect in rects.borrow().iter() {
                let middle = rect.origin.y + rect.size.height / 2.0;
                let layer = CALayer::new();
                layer.setFrame(CGRect::new(
                    CGPoint::new(rect.origin.x, height - middle - fill_h / 2.0),
                    CGSize::new(rect.size.width, fill_h),
                ));
                layer.setBackgroundColor(Some(&theme::cg(p.new_fill)));
                layer.setCornerRadius(theme::CORNER_INLINE);
                root.addSublayer(&layer);
            }
        }
    });
    view.addSubview(&backdrop);

    let words =
        NSTextView::initWithFrame_textContainer(NSTextView::alloc(mtm), frame, Some(&container));
    words.setEditable(false);
    words.setSelectable(false);
    words.setDrawsBackground(false);
    words.setTextContainerInset(CGSize::new(0.0, 0.0));
    view.addSubview(&words);
    (view, height)
}

/// One stretch of the sentence with its attributes.
fn run(
    text: &str,
    font: &objc2_app_kit::NSFont,
    color: Rgba,
    struck: bool,
    raise: f64,
    style: &NSMutableParagraphStyle,
) -> Retained<NSAttributedString> {
    let color = theme::ns(color);
    let one = NSNumber::new_isize(1);
    let raise = NSNumber::new_f64(raise);
    let mut keys: Vec<&NSAttributedStringKey> = unsafe {
        vec![
            NSFontAttributeName,
            NSForegroundColorAttributeName,
            NSParagraphStyleAttributeName,
            NSBaselineOffsetAttributeName,
        ]
    };
    let mut values: Vec<&AnyObject> = vec![font, &color, style, &raise];
    if struck {
        keys.push(unsafe { NSStrikethroughStyleAttributeName });
        keys.push(unsafe { NSStrikethroughColorAttributeName });
        values.push(&one);
        values.push(&color);
    }
    let attributes = NSDictionary::from_slices(&keys, &values);
    unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &NSString::from_str(text),
            Some(&attributes),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_match_the_tokens() {
        use crate::macos::theme::tokens::{load, num};
        let t = load();
        for (path, ours) in [
            ("card.width", WIDTH),
            ("card.padding", PAD),
            ("card.under_title", UNDER_TITLE),
            ("card.above_buttons", ABOVE_BUTTONS),
            ("card.between_buttons", BETWEEN),
            ("card.context_words", CONTEXT as f64),
            ("button.height", BUTTON_H),
            ("button.side", BUTTON_SIDE),
            ("type.sentence.size", TEXT),
            ("type.sentence.line", LINE),
            ("underline.hover_fill", HOVER_FILL),
            ("shadow.tight.inset", SHADOW_TIGHT.0),
            ("shadow.tight.drop", SHADOW_TIGHT.1),
            ("shadow.tight.blur", SHADOW_TIGHT.2),
            ("shadow.tight.share", f64::from(SHADOW_TIGHT.3)),
            ("shadow.soft.inset", SHADOW_SOFT.0),
            ("shadow.soft.drop", SHADOW_SOFT.1),
            ("shadow.soft.blur", SHADOW_SOFT.2),
            ("shadow.soft.share", f64::from(SHADOW_SOFT.3)),
            ("motion.card_fade_ms", FADE * 1000.0),
            ("motion.card_rise", RISE),
            (
                "motion.accepted_ms",
                crate::macos::app::FLASH.as_millis() as f64,
            ),
        ] {
            let theirs = num(&t, path);
            assert!(
                (ours - theirs).abs() < 1e-6,
                "{path}: {ours} here, {theirs} in tokens.json"
            );
        }
    }

    fn fix(text: &str, gone: &str, replacement: &str) -> Fix {
        let byte = text.rfind(gone).unwrap();
        let start = text[..byte].chars().count();
        Fix {
            start,
            end: start + gone.chars().count(),
            replacement: replacement.into(),
            kind: Kind::Correctness,
            message: String::new(),
            source: Source::Ai,
        }
    }

    fn same(s: &str) -> Bit {
        Bit::Same(s.into())
    }

    #[test]
    fn a_word_is_shown_in_its_sentence() {
        let text = "Did you recieve the photos?";
        let f = fix(text, "recieve", "receive");
        assert_eq!(
            snippet(text, &[&f]),
            vec![
                same("Did you "),
                Bit::Gone("recieve".into()),
                same(" "),
                Bit::New("receive".into()),
                same(" the photos?"),
            ]
        );
    }

    #[test]
    fn a_removal_is_struck_where_it_was() {
        let text = "Can you send me the the link for the frame.io draft when you have a sec?";
        let byte = text.find("the the").unwrap() + 4;
        let start = text[..byte].chars().count();
        let f = Fix {
            start,
            end: start + 4,
            replacement: String::new(),
            kind: Kind::Correctness,
            message: String::new(),
            source: Source::Ai,
        };
        assert_eq!(
            snippet(text, &[&f]),
            vec![
                same("Can you send me the "),
                Bit::Gone("the".into()),
                same(" link for the frame.io draft when…"),
            ]
        );
    }

    #[test]
    fn a_long_sentence_is_cut_around_the_change() {
        let text = "One two three four five six seven eight nine tehn eleven twelve thirteen fourteen fifteen sixteen seventeen.";
        let f = fix(text, "tehn", "ten");
        let bits = snippet(text, &[&f]);
        assert_eq!(
            bits.first(),
            Some(&same("…four five six seven eight nine "))
        );
        assert_eq!(
            bits.last(),
            Some(&same(" eleven twelve thirteen fourteen fifteen sixteen…"))
        );
    }

    #[test]
    fn a_change_of_spaces_alone_is_visible() {
        let text = "yes  was";
        let f = fix(text, "  ", " ");
        assert_eq!(
            snippet(text, &[&f]),
            vec![same("yes "), Bit::Gone(SPACE.into()), same(" was")]
        );
    }

    #[test]
    fn a_rewrite_is_not_shown_word_by_word() {
        let text = "I think it would maybe be better if we could try to move it.";
        let mut f = fix(
            text,
            "it would maybe be better if we could try to",
            "we should",
        );
        f.kind = Kind::Clarity;
        assert_eq!(
            snippet(text, &[&f]),
            vec![
                same("I think "),
                Bit::Gone("it would maybe be better if we could try to".into()),
                same(" "),
                Bit::New("we should".into()),
                same(" move it."),
            ]
        );
    }

    #[test]
    fn several_fixes_share_one_sentence() {
        let text = "i went to paris on monday and it was great.";
        let fixes = [
            fix(text, "i went", "I went"),
            fix(text, "paris", "Paris"),
            fix(text, "monday", "Monday"),
        ];
        let refs: Vec<&Fix> = fixes.iter().collect();
        let bits = snippet(text, &refs);
        let news: Vec<&Bit> = bits.iter().filter(|b| matches!(b, Bit::New(_))).collect();
        assert_eq!(
            news,
            vec![
                &Bit::New("I".into()),
                &Bit::New("Paris".into()),
                &Bit::New("Monday".into())
            ]
        );
        assert_eq!(bits.last(), Some(&same(" and it was great.")));
    }
}
