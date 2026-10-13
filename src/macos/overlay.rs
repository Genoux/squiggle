//! What Gramly draws on top of other apps: underlines, the suggestion card,
//! and the Accept-all pill.
//!
//! All three are non-activating panels. Clicking Accept must not take focus
//! away from the field being corrected - an activating window would, the
//! caret would leave the text, and the user would have to click back in to
//! keep typing. The underline panel also ignores the mouse entirely, so
//! clicks land on the text beneath it exactly as if it were not there.

use crate::macos::theme;
use gramly_core::{Fix, Kind};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSBackingStoreType,
    NSButton, NSColor, NSFont, NSFontWeightMedium, NSFontWeightRegular, NSFontWeightSemibold,
    NSLineBreakMode, NSPanel, NSScreen, NSStatusWindowLevel, NSTextField, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::NSString;
use objc2_quartz_core::CALayer;

/// One underline, already in Cocoa screen coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub rect: CGRect,
    pub kind: Kind,
    pub highlighted: bool,
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

pub fn contains(rect: CGRect, point: CGPoint) -> bool {
    point.x >= rect.origin.x
        && point.x <= rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y <= rect.origin.y + rect.size.height
}

fn panel(mtm: MainThreadMarker, interactive: bool) -> Retained<NSPanel> {
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
    panel.setHasShadow(interactive);
    panel.setIgnoresMouseEvents(!interactive);
    panel.setLevel(NSStatusWindowLevel);
    // An accessory app is never active, and a panel that hides on
    // deactivation would therefore never show at all.
    panel.setHidesOnDeactivate(false);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setFloatingPanel(true);
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    if let Some(dark) = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }) {
        panel.setAppearance(Some(&dark));
    }
    let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
    view.setWantsLayer(true);
    panel.setContentView(Some(&view));
    panel
}

fn layer_of(panel: &NSPanel) -> Option<Retained<CALayer>> {
    panel.contentView().and_then(|view| view.layer())
}

fn surface(layer: &CALayer, radius: f64) {
    layer.setBackgroundColor(Some(&theme::cg(theme::SURFACE)));
    layer.setCornerRadius(radius);
    layer.setBorderWidth(1.0);
    layer.setBorderColor(Some(&theme::cg(theme::EDGE)));
}

fn label(
    text: &str,
    size: f64,
    weight: f64,
    color: theme::Rgba,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&NSFont::systemFontOfSize_weight(size, weight)));
    field.setTextColor(Some(&theme::ns(color)));
    field
}

fn button(
    title: &str,
    target: &AnyObject,
    action: Sel,
    tag: isize,
    primary: bool,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setTag(tag);
    if primary {
        button.setBezelColor(Some(&theme::ns(theme::ACCENT)));
    }
    button
}

pub struct Overlay {
    mtm: MainThreadMarker,
    marks_panel: Retained<NSPanel>,
    drawn: Vec<Mark>,
    card: Retained<NSPanel>,
    /// The fix the card is showing, and where.
    pub card_for: Option<usize>,
    pill: Retained<NSPanel>,
    pill_count: usize,
}

impl Overlay {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            mtm,
            marks_panel: panel(mtm, false),
            drawn: Vec::new(),
            card: panel(mtm, true),
            card_for: None,
            pill: panel(mtm, true),
            pill_count: 0,
        }
    }

    /// Redraws the underlines only when they changed: rebuilding layers on
    /// every tick would cost a compositor frame ten times a second for nothing.
    pub fn marks(&mut self, area: CGRect, marks: Vec<Mark>) {
        if marks.is_empty() {
            self.hide_marks();
            return;
        }
        if self.drawn == marks && self.marks_panel.isVisible() && self.marks_panel.frame() == area {
            return;
        }
        self.marks_panel.setFrame_display(area, false);
        let Some(root) = layer_of(&self.marks_panel) else {
            return;
        };
        unsafe { root.setSublayers(None) };
        for mark in &marks {
            let x = mark.rect.origin.x - area.origin.x;
            let y = mark.rect.origin.y - area.origin.y;
            let color = theme::kind(mark.kind);
            if mark.highlighted {
                let fill = CALayer::new();
                fill.setFrame(CGRect::new(
                    CGPoint::new(x - 1.0, y),
                    CGSize::new(mark.rect.size.width + 2.0, mark.rect.size.height),
                ));
                fill.setBackgroundColor(Some(&theme::cg(theme::Rgba { a: 0.22, ..color })));
                fill.setCornerRadius(3.0);
                root.addSublayer(&fill);
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
        if std::env::var_os("GRAMLY_TRACE").is_some() {
            eprintln!("gramly: drawing {} underlines in {area:?}", marks.len());
        }
        self.drawn = marks;
        self.marks_panel.orderFrontRegardless();
    }

    pub fn hide_marks(&mut self) {
        if self.marks_panel.isVisible() {
            self.marks_panel.orderOut(None);
        }
        self.drawn.clear();
    }

    pub fn card_frame(&self) -> Option<CGRect> {
        self.card.isVisible().then(|| self.card.frame())
    }

    /// The card for one fix, placed under the word it is about - or above it
    /// when the word sits too close to the bottom of the screen.
    pub fn show_card(
        &mut self,
        index: usize,
        fix: &Fix,
        original: &str,
        word: CGRect,
        target: &AnyObject,
    ) {
        if self.card_for == Some(index) && self.card.isVisible() {
            return;
        }
        let mtm = self.mtm;
        let width = 300.0;
        let pad = 16.0;
        let inner = width - pad * 2.0;

        let heading = match fix.kind {
            Kind::Spelling => "Spelling",
            Kind::Correctness => "Correctness",
            Kind::Clarity => "Clarity",
        };
        let change = if fix.replacement.is_empty() {
            format!("Remove “{original}”")
        } else if original.is_empty() {
            format!("Add “{}”", fix.replacement)
        } else {
            format!("{original}  →  {}", fix.replacement)
        };

        let heading = label(
            heading,
            11.0,
            unsafe { NSFontWeightMedium },
            theme::kind(fix.kind),
            mtm,
        );
        let change = label(
            &change,
            15.0,
            unsafe { NSFontWeightSemibold },
            theme::TEXT,
            mtm,
        );
        change.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        let message = label(
            &fix.message,
            12.0,
            unsafe { NSFontWeightRegular },
            theme::MUTED,
            mtm,
        );
        message.setMaximumNumberOfLines(3);
        message.setPreferredMaxLayoutWidth(inner);
        let message_height = message
            .sizeThatFits(CGSize::new(inner, 200.0))
            .height
            .ceil();

        let accept = button(
            "Accept",
            target,
            objc2::sel!(accept:),
            index as isize,
            true,
            mtm,
        );
        let dismiss = button(
            "Dismiss",
            target,
            objc2::sel!(dismiss:),
            index as isize,
            false,
            mtm,
        );
        let learn = (fix.kind == Kind::Spelling).then(|| {
            button(
                "Add to dictionary",
                target,
                objc2::sel!(learn:),
                index as isize,
                false,
                mtm,
            )
        });

        // Laid out top to bottom, then flipped into Cocoa's bottom-up frame.
        let buttons_h = 28.0;
        let height = pad + 14.0 + 6.0 + 20.0 + 6.0 + message_height + 14.0 + buttons_h + pad;
        let mut top = height - pad;
        let mut place = |view: &NSView, h: f64, x: f64, w: f64, gap: f64| {
            top -= h;
            view.setFrame(CGRect::new(CGPoint::new(x, top), CGSize::new(w, h)));
            top -= gap;
        };
        place(&heading, 14.0, pad, inner, 6.0);
        place(&change, 20.0, pad, inner, 6.0);
        place(&message, message_height, pad, inner, 14.0);

        let mut x = pad;
        for b in [Some(&accept), Some(&dismiss), learn.as_ref()]
            .into_iter()
            .flatten()
        {
            let w = b.fittingSize().width.max(72.0);
            b.setFrame(CGRect::new(CGPoint::new(x, pad), CGSize::new(w, buttons_h)));
            x += w + 8.0;
        }

        let frame0 = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height));
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame0);
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            surface(&layer, 12.0);
        }
        for sub in [&*heading as &NSView, &change, &message, &accept, &dismiss] {
            view.addSubview(sub);
        }
        if let Some(learn) = &learn {
            view.addSubview(learn);
        }
        self.card.setContentView(Some(&view));

        let gap = 6.0;
        let mut origin = CGPoint::new(word.origin.x - pad, word.origin.y - gap - height);
        if origin.y < 0.0 {
            origin.y = word.origin.y + word.size.height + gap;
        }
        self.card
            .setFrame_display(CGRect::new(origin, CGSize::new(width, height)), true);
        self.card.orderFrontRegardless();
        self.card_for = Some(index);
    }

    pub fn hide_card(&mut self) {
        if self.card.isVisible() {
            self.card.orderOut(None);
        }
        self.card_for = None;
    }

    /// "3 · Accept all", tucked inside the field's bottom-right corner the way
    /// Grammarly's badge sits.
    pub fn pill(&mut self, field: CGRect, count: usize, target: &AnyObject) {
        if count == 0 {
            self.hide_pill();
            return;
        }
        let mtm = self.mtm;
        let height = 26.0;
        if count != self.pill_count || !self.pill.isVisible() {
            let title = if count == 1 {
                "1 fix · Accept".to_string()
            } else {
                format!("{count} fixes · Accept all")
            };
            let accept = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(&title),
                    Some(target),
                    Some(objc2::sel!(acceptAll:)),
                    mtm,
                )
            };
            accept.setBordered(false);
            accept.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, unsafe {
                NSFontWeightMedium
            })));
            accept.setContentTintColor(Some(&theme::ns(theme::TEXT)));
            let width = accept.fittingSize().width + 24.0;
            let frame0 = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height));
            let view = NSView::initWithFrame(NSView::alloc(mtm), frame0);
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                surface(&layer, height / 2.0);
            }
            let dot = CALayer::new();
            dot.setFrame(CGRect::new(
                CGPoint::new(10.0, height / 2.0 - 3.0),
                CGSize::new(6.0, 6.0),
            ));
            dot.setCornerRadius(3.0);
            dot.setBackgroundColor(Some(&theme::cg(theme::ACCENT)));
            if let Some(layer) = view.layer() {
                layer.addSublayer(&dot);
            }
            accept.setFrame(CGRect::new(
                CGPoint::new(14.0, 0.0),
                CGSize::new(width - 18.0, height),
            ));
            view.addSubview(&accept);
            self.pill.setContentView(Some(&view));
            self.pill_count = count;
        }
        let width = self
            .pill
            .contentView()
            .map(|v| v.frame().size.width)
            .unwrap_or(120.0);
        let origin = CGPoint::new(
            field.origin.x + field.size.width - width - 8.0,
            field.origin.y + 6.0,
        );
        let frame = CGRect::new(origin, CGSize::new(width, height));
        if self.pill.frame() != frame || !self.pill.isVisible() {
            self.pill.setFrame_display(frame, true);
            self.pill.orderFrontRegardless();
        }
    }

    pub fn hide_pill(&mut self) {
        if self.pill.isVisible() {
            self.pill.orderOut(None);
        }
        self.pill_count = 0;
    }

    pub fn hide_all(&mut self) {
        self.hide_marks();
        self.hide_card();
        self.hide_pill();
    }
}
