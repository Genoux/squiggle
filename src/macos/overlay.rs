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

use crate::macos::primary_height;
use crate::macos::theme::{
    self, ABOVE_BUTTONS, BETWEEN_BUTTONS, BUTTON_HEIGHT, BUTTON_OUTLINE, BUTTON_SIDE, CARD_PAD,
    CARD_RISE, CARD_TITLE, CARD_WIDTH, DOT, EDGE_WIDTH, Face, NEW_WORD_PAD, NEW_WORDS, Palette,
    Rgba, SENTENCE, SENTENCE_LINE, SHADE, SHADOW_SOFT, SHADOW_TIGHT, Shadow, TITLE_INDENT,
    TITLE_LINE, UNDER_TITLE, UNDERLINE_BLEED, UNDERLINE_HEIGHT, UNDERLINE_OFFSET,
};
use crate::surface::{self, Action, Bit, Card, Mark, Point, Rect, place_card};
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

/// Accessibility reports rectangles from the top-left of the primary display
/// with y growing down; Cocoa places windows from its bottom-left with y
/// growing up. Every rectangle crosses this once, here, and the flip is its
/// own inverse.
fn to_cocoa(rect: Rect, mtm: MainThreadMarker) -> CGRect {
    CGRect::new(
        CGPoint::new(rect.x, primary_height(mtm) - rect.y - rect.h),
        CGSize::new(rect.w, rect.h),
    )
}

fn from_cocoa(rect: CGRect, mtm: MainThreadMarker) -> Rect {
    Rect {
        x: rect.origin.x,
        y: primary_height(mtm) - rect.origin.y - rect.size.height,
        w: rect.size.width,
        h: rect.size.height,
    }
}

/// The usable part (no menu bar, no Dock) of the screen a rectangle is on;
/// the main screen's when it is on none.
fn screen_around(rect: Rect, mtm: MainThreadMarker) -> Rect {
    let middle = Point {
        x: rect.x + rect.w / 2.0,
        y: rect.y + rect.h / 2.0,
    };
    let screens = NSScreen::screens(mtm);
    screens
        .iter()
        .find(|screen| from_cocoa(screen.frame(), mtm).contains(middle))
        .or_else(|| NSScreen::mainScreen(mtm))
        .map(|screen| from_cocoa(screen.visibleFrame(), mtm))
        .unwrap_or(rect)
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

pub struct Overlay {
    mtm: MainThreadMarker,
    /// What the card's buttons send their clicks to. The controller owns the
    /// engine that owns this, so the two keep each other alive: one of each
    /// exists for the life of the process.
    target: Retained<AnyObject>,
    marks_panel: Retained<NSPanel>,
    drawn: Vec<Mark>,
    card: Retained<NSPanel>,
    /// The card's shadow, in a panel of its own that ignores the mouse: in
    /// the card's panel, the shadow's pixels would take clicks meant for the
    /// text below the card.
    shade: Retained<NSPanel>,
    /// The card's buttons, and which one the pointer is over.
    rows: Vec<Row>,
    lit: Option<usize>,
}

impl Overlay {
    pub fn new(mtm: MainThreadMarker, target: Retained<AnyObject>) -> Self {
        Self {
            mtm,
            target,
            marks_panel: panel(mtm, false, NSStatusWindowLevel),
            drawn: Vec::new(),
            // One level above the underlines, so a line from the text under
            // the card never crosses it.
            card: panel(mtm, true, NSStatusWindowLevel + 1),
            shade: panel(mtm, false, NSStatusWindowLevel),
            rows: Vec::new(),
            lit: None,
        }
    }
}

impl surface::Overlay for Overlay {
    /// Redraws the underlines only when they changed: rebuilding layers on
    /// every tick would cost a compositor frame twenty times a second for
    /// nothing.
    fn marks(&mut self, area: Rect, marks: Vec<Mark>) {
        if marks.is_empty() {
            self.hide_marks();
            return;
        }
        let area = to_cocoa(area, self.mtm);
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
                let rect = to_cocoa(mark.rect, self.mtm);
                let x = rect.origin.x - area.origin.x;
                let y = rect.origin.y - area.origin.y;
                if let Some(fill) = theme::mark_fill(mark) {
                    let layer = CALayer::new();
                    layer.setFrame(CGRect::new(
                        CGPoint::new(x - UNDERLINE_BLEED, y),
                        CGSize::new(rect.size.width + UNDERLINE_BLEED * 2.0, rect.size.height),
                    ));
                    layer.setBackgroundColor(Some(&theme::cg(fill)));
                    layer.setCornerRadius(theme::CORNER_INLINE);
                    root.addSublayer(&layer);
                }
                let Some(color) = theme::mark_line(mark) else {
                    continue;
                };
                let line = CALayer::new();
                line.setFrame(CGRect::new(
                    CGPoint::new(x, y - UNDERLINE_OFFSET),
                    CGSize::new(rect.size.width, UNDERLINE_HEIGHT),
                ));
                line.setBackgroundColor(Some(&theme::cg(color)));
                line.setCornerRadius(UNDERLINE_HEIGHT / 2.0);
                root.addSublayer(&line);
            }
        });
        self.drawn = marks;
        self.marks_panel.orderFrontRegardless();
    }

    fn hide_marks(&mut self) {
        if self.marks_panel.isVisible() {
            self.marks_panel.orderOut(None);
        }
        self.drawn.clear();
    }

    fn card_rows(&self) -> Vec<(Action, Rect)> {
        if !self.card.isVisible() {
            return Vec::new();
        }
        let origin = self.card.frame().origin;
        self.rows
            .iter()
            .filter_map(|row| {
                let f = row.view.frame();
                let on_screen = CGRect::new(
                    CGPoint::new(origin.x + f.origin.x, origin.y + f.origin.y),
                    f.size,
                );
                row.action
                    .map(|action| (action, from_cocoa(on_screen, self.mtm)))
            })
            .collect()
    }

    fn card_frame(&self) -> Option<Rect> {
        self.card
            .isVisible()
            .then(|| from_cocoa(self.card.frame(), self.mtm))
    }

    fn show_card(&mut self, index: usize, content: &Card, word: Rect) {
        let palette = theme::palette(self.mtm);
        let built = build(index, content, &self.target, palette, self.mtm);
        self.rows = built.rows;
        self.lit = None;
        let size = built.view.frame().size;
        self.card.setContentView(Some(&built.view));

        let area = screen_around(word, self.mtm);
        let origin = place_card(word, (size.width, size.height), area);
        let frame = Rect {
            x: origin.x,
            y: origin.y,
            w: size.width,
            h: size.height,
        };
        let shade = shade(size, palette, self.mtm);
        let cocoa = to_cocoa(frame, self.mtm);
        self.shade.setContentView(Some(&shade));
        self.shade.setFrame_display(
            CGRect::new(
                CGPoint::new(cocoa.origin.x - SHADE.0, cocoa.origin.y - SHADE.3),
                shade.frame().size,
            ),
            true,
        );
        self.card.setFrame_display(cocoa, true);
        // Settling towards its word: down onto a card below it, up onto
        // one above.
        let from = if frame.y >= word.y + word.h {
            CARD_RISE
        } else {
            -CARD_RISE
        };
        fade_in(&shade, from);
        fade_in(&built.view, from);
        self.shade.orderFrontRegardless();
        self.card.orderFrontRegardless();
    }

    fn hide_card(&mut self) {
        if self.card.isVisible() {
            self.card.orderOut(None);
        }
        if self.shade.isVisible() {
            self.shade.orderOut(None);
        }
        self.rows.clear();
        self.lit = None;
    }

    /// Lights the button under the pointer. Driven by the same poll as the
    /// underlines: the card never becomes key, so it gets no tracking-area
    /// events of its own.
    fn hover(&mut self, pointer: Point) {
        if !self.card.isVisible() {
            return;
        }
        let origin = self.card.frame().origin;
        let at = Point {
            x: pointer.x - origin.x,
            y: primary_height(self.mtm) - pointer.y - origin.y,
        };
        let now = self
            .rows
            .iter()
            .position(|row| Rect::from(row.view.frame()).contains(at));
        if now == self.lit {
            return;
        }
        CATransaction::begin();
        CATransaction::setAnimationDuration(theme::HOVER_FADE.as_secs_f64());
        for (i, row) in self.rows.iter().enumerate() {
            row.light(Some(i) == now);
        }
        CATransaction::commit();
        self.lit = now;
    }
}

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
        animation.setDuration(theme::CARD_FADE.as_secs_f64());
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
    /// What clicking it does, for `GRAMLY_STATE`; none for a button of
    /// another window.
    action: Option<Action>,
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

fn label(text: &str, face: Face, color: Rgba, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&theme::font(face.size, face.weight)));
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
    selector: Sel,
    action: Option<Action>,
    tag: isize,
    target: &AnyObject,
    p: &Palette,
    mtm: MainThreadMarker,
) -> Row {
    let caption = if primary {
        label(words, theme::BUTTON_MAIN, p.on_ink, mtm)
    } else {
        label(words, theme::BUTTON, p.text, mtm)
    };
    let fit = caption.fittingSize();
    let width = fit.width.ceil() + BUTTON_SIDE * 2.0;
    let view = layer_view(
        CGRect::new(CGPoint::new(x, y), CGSize::new(width, BUTTON_HEIGHT)),
        mtm,
    );
    caption.setFrame(CGRect::new(
        CGPoint::new(BUTTON_SIDE, ((BUTTON_HEIGHT - fit.height) / 2.0).round()),
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
                layer.setBorderWidth(BUTTON_OUTLINE);
            }
        }
        row.light(false);
    });
    let cover = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(""),
            Some(target),
            Some(selector),
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
    selector: Sel,
    target: &AnyObject,
    p: &Palette,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    button(words, primary, 0.0, 0.0, selector, None, 0, target, p, mtm).view
}

/// What a button's click is sent to, in the controller.
fn selector(action: Action) -> Sel {
    match action {
        Action::Accept => sel!(accept:),
        Action::Dismiss => sel!(dismiss:),
        Action::Learn => sel!(learn:),
        Action::AcceptSentence => sel!(acceptSentence:),
        Action::DismissSentence => sel!(dismissSentence:),
    }
}

pub(crate) struct Built {
    pub view: Retained<NSView>,
    rows: Vec<Row>,
}

/// Builds the card for the fix at `index`.
pub(crate) fn build(
    index: usize,
    content: &Card,
    target: &AnyObject,
    p: &Palette,
    mtm: MainThreadMarker,
) -> Built {
    let tag = index as isize;
    let dot = theme::kind(content.kind);
    let inner = theme::CARD_WIDTH - theme::CARD_PAD * 2.0;
    let gone = theme::gone(p, content.kind);
    let (sentence, sentence_h) = sentence_view(&content.bits, inner, gone, p, mtm);

    // Buttons left to right, wrapping in the rare case they do not fit.
    let mut placed: Vec<(Row, usize)> = Vec::new();
    let mut line = 0;
    let mut x = CARD_PAD;
    for (i, &action) in content.actions.iter().enumerate() {
        let row = button(
            action.label(),
            i == 0,
            0.0,
            0.0,
            selector(action),
            Some(action),
            tag,
            target,
            p,
            mtm,
        );
        let w = row.view.frame().size.width;
        if x > CARD_PAD && x + w > CARD_WIDTH - CARD_PAD {
            line += 1;
            x = CARD_PAD;
        }
        row.view.setFrameOrigin(CGPoint::new(x, 0.0));
        x += w + BETWEEN_BUTTONS;
        placed.push((row, line));
    }
    let lines = (line + 1) as f64;
    let buttons_h = lines * BUTTON_HEIGHT + (lines - 1.0) * BETWEEN_BUTTONS;
    let height =
        CARD_PAD + TITLE_LINE + UNDER_TITLE + sentence_h + ABOVE_BUTTONS + buttons_h + CARD_PAD;

    let view = layer_view(
        CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(CARD_WIDTH, height)),
        mtm,
    );
    still(|| {
        if let Some(layer) = view.layer() {
            layer.setBackgroundColor(Some(&theme::cg(p.surface)));
            layer.setCornerRadius(theme::CORNER);
            layer.setBorderWidth(EDGE_WIDTH);
            layer.setBorderColor(Some(&theme::cg(p.edge)));
        }
    });

    let mut top = height - CARD_PAD;
    let marker = layer_view(
        CGRect::new(
            CGPoint::new(CARD_PAD, top - TITLE_LINE / 2.0 - DOT / 2.0),
            CGSize::new(DOT, DOT),
        ),
        mtm,
    );
    still(|| {
        if let Some(layer) = marker.layer() {
            layer.setBackgroundColor(Some(&theme::cg(dot)));
            layer.setCornerRadius(DOT / 2.0);
        }
    });
    view.addSubview(&marker);
    let heading = label(&content.title, CARD_TITLE, p.text2, mtm);
    let heading_h = heading.fittingSize().height;
    heading.setFrame(CGRect::new(
        CGPoint::new(
            CARD_PAD + TITLE_INDENT,
            top - TITLE_LINE + ((TITLE_LINE - heading_h) / 2.0).round(),
        ),
        CGSize::new(inner - TITLE_INDENT, heading_h),
    ));
    view.addSubview(&heading);
    top -= TITLE_LINE + UNDER_TITLE + sentence_h;
    sentence.setFrameOrigin(CGPoint::new(CARD_PAD, top));
    view.addSubview(&sentence);

    let mut rows = Vec::new();
    for (row, line) in placed {
        let y =
            CARD_PAD + buttons_h - BUTTON_HEIGHT - line as f64 * (BUTTON_HEIGHT + BETWEEN_BUTTONS);
        row.view
            .setFrameOrigin(CGPoint::new(row.view.frame().origin.x, y));
        view.addSubview(&row.view);
        rows.push(row);
    }
    Built { view, rows }
}

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
        for Shadow {
            inset,
            drop,
            blur,
            share,
        } in [SHADOW_TIGHT, SHADOW_SOFT]
        {
            let strength = (f64::from(p.shadow) * share) as f32;
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
    let regular = theme::font(SENTENCE.size, SENTENCE.weight);
    let bold = theme::font(NEW_WORDS.size, NEW_WORDS.weight);
    // Centred in its 24pt line: fixed line heights put the extra room above
    // the letters.
    let natural = regular.ascender() - regular.descender();
    let raise = ((SENTENCE_LINE - natural) / 2.0).max(0.0);
    let style = NSMutableParagraphStyle::new();
    style.setMinimumLineHeight(SENTENCE_LINE);
    style.setMaximumLineHeight(SENTENCE_LINE);

    let text = NSMutableAttributedString::new();
    let mut fills: Vec<NSRange> = Vec::new();
    for bit in bits {
        let (shown, font, color, struck) = match bit {
            Bit::Same(s) => (s.clone(), &regular, p.text, false),
            Bit::Gone(s) => (s.clone(), &regular, gone, true),
            Bit::New(s) => (
                format!("{NEW_WORD_PAD}{s}{NEW_WORD_PAD}"),
                &bold,
                p.new,
                false,
            ),
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
    let height = used.size.height.ceil().max(SENTENCE_LINE);

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
    let fill_h = theme::new_word_fill_height(natural);
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
