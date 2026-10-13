//! What Gramly draws on top of other apps: underlines, the suggestion card,
//! and the Accept-all pill.
//!
//! All three are non-activating panels. Clicking Accept must not take focus
//! away from the field being corrected - an activating window would, the
//! caret would leave the text, and the user would have to click back in to
//! keep typing. The underline panel also ignores the mouse entirely, so
//! clicks land on the text beneath it exactly as if it were not there.
//!
//! Nothing animates. Core Animation gives every new layer an implicit
//! position animation, which drew each underline sliding in from the panel's
//! corner; every layer change here sits inside a transaction with actions
//! disabled, and the panels have no window animation either.

use crate::macos::theme;
use gramly_core::{Fix, Kind, Source};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSBackingStoreType,
    NSButton, NSColor, NSFont, NSFontAttributeName, NSFontWeightMedium, NSFontWeightRegular,
    NSFontWeightSemibold, NSForegroundColorAttributeName, NSLineBreakMode, NSPanel, NSScreen,
    NSStatusWindowLevel, NSStrikethroughColorAttributeName, NSStrikethroughStyleAttributeName,
    NSTextField, NSView, NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableAttributedString, NSNumber,
    NSString,
};
use objc2_quartz_core::{CALayer, CATransaction};

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

fn still<T>(change: impl FnOnce() -> T) -> T {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    let result = change();
    CATransaction::commit();
    result
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
    panel.setAnimationBehavior(NSWindowAnimationBehavior::None);
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

fn surface(layer: &CALayer, radius: f64) {
    layer.setBackgroundColor(Some(&theme::cg(theme::SURFACE)));
    layer.setCornerRadius(radius);
    layer.setBorderWidth(1.0);
    layer.setBorderColor(Some(&theme::cg(theme::EDGE)));
}

/// One run of styled text.
struct Run<'a> {
    text: &'a str,
    color: theme::Rgba,
    size: f64,
    weight: f64,
    struck: bool,
}

fn styled(runs: &[Run]) -> Retained<NSMutableAttributedString> {
    let out = NSMutableAttributedString::new();
    for run in runs {
        let font = NSFont::systemFontOfSize_weight(run.size, run.weight);
        let color = theme::ns(run.color);
        let one = NSNumber::new_isize(1);
        let mut keys: Vec<&NSAttributedStringKey> =
            unsafe { vec![NSFontAttributeName, NSForegroundColorAttributeName] };
        let mut values: Vec<&AnyObject> = vec![&font, &color];
        if run.struck {
            keys.push(unsafe { NSStrikethroughStyleAttributeName });
            keys.push(unsafe { NSStrikethroughColorAttributeName });
            values.push(&one);
            values.push(&color);
        }
        let attributes = NSDictionary::from_slices(&keys, &values);
        let piece = unsafe {
            NSAttributedString::initWithString_attributes(
                NSAttributedString::alloc(),
                &NSString::from_str(run.text),
                Some(&attributes),
            )
        };
        out.appendAttributedString(&piece);
    }
    out
}

fn label(runs: &[Run], mtm: MainThreadMarker) -> Retained<NSTextField> {
    NSTextField::labelWithAttributedString(&styled(runs), mtm)
}

/// A flat button: a filled one for Accept, plain text for the rest, as the
/// card in Grammarly draws them - a row of bezelled system buttons read as a
/// dialog, not as a suggestion.
fn flat(
    title: &str,
    filled: bool,
    target: &AnyObject,
    action: Sel,
    tag: isize,
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
    button.setBordered(false);
    let color = if filled {
        theme::ON_ACCENT
    } else {
        theme::MUTED
    };
    let weight = if filled {
        unsafe { NSFontWeightSemibold }
    } else {
        unsafe { NSFontWeightMedium }
    };
    button.setAttributedTitle(&styled(&[Run {
        text: title,
        color,
        size: 13.0,
        weight,
        struck: false,
    }]));
    if filled {
        button.setWantsLayer(true);
        if let Some(layer) = button.layer() {
            layer.setBackgroundColor(Some(&theme::cg(theme::ACCENT)));
            layer.setCornerRadius(6.0);
        }
    }
    button
}

pub struct Overlay {
    mtm: MainThreadMarker,
    marks_panel: Retained<NSPanel>,
    drawn: Vec<Mark>,
    card: Retained<NSPanel>,
    /// The fix the card is showing.
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
            eprintln!("gramly: drawing {} underlines in {area:?}", marks.len());
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
                if mark.highlighted {
                    let fill = CALayer::new();
                    fill.setFrame(CGRect::new(
                        CGPoint::new(x - 1.0, y),
                        CGSize::new(mark.rect.size.width + 2.0, mark.rect.size.height),
                    ));
                    fill.setBackgroundColor(Some(&theme::cg(theme::Rgba { a: 0.2, ..color })));
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

    pub fn card_frame(&self) -> Option<CGRect> {
        self.card.isVisible().then(|| self.card.frame())
    }

    /// The card for the hovered suggestion. One fix in its sentence gets the
    /// small card - a title, the replacement to click, Dismiss. Two or more
    /// get the sentence card: the whole sentence with every change in place
    /// and one "Correct the sentence" button.
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
        let Some((_, fix)) = group.iter().find(|(i, _)| *i == index) else {
            return;
        };
        let view = if group.len() > 1 {
            sentence_card(index, group, text, target, self.mtm)
        } else {
            word_card(index, fix, text, target, self.mtm)
        };
        let size = view.frame().size;
        self.card.setContentView(Some(&view));

        let gap = 8.0;
        let mut origin = CGPoint::new(word.origin.x - 12.0, word.origin.y - gap - size.height);
        if origin.y < 0.0 {
            origin.y = word.origin.y + word.size.height + gap;
        }
        self.card.setFrame_display(CGRect::new(origin, size), true);
        self.card.orderFrontRegardless();
        self.card_for = Some(index);
    }

    pub fn hide_card(&mut self) {
        if self.card.isVisible() {
            self.card.orderOut(None);
        }
        self.card_for = None;
    }

    /// "3 fixes · Accept all", tucked inside the field's bottom-right corner
    /// the way Grammarly's badge sits.
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
            accept.setAttributedTitle(&styled(&[Run {
                text: &title,
                color: theme::TEXT,
                size: 12.0,
                weight: unsafe { NSFontWeightMedium },
                struck: false,
            }]));
            let width = accept.fittingSize().width + 30.0;
            let view = NSView::initWithFrame(
                NSView::alloc(mtm),
                CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height)),
            );
            view.setWantsLayer(true);
            accept.setFrame(CGRect::new(
                CGPoint::new(18.0, 0.0),
                CGSize::new(width - 24.0, height),
            ));
            still(|| {
                if let Some(layer) = view.layer() {
                    surface(&layer, height / 2.0);
                    let dot = CALayer::new();
                    dot.setFrame(CGRect::new(
                        CGPoint::new(11.0, height / 2.0 - 3.0),
                        CGSize::new(6.0, 6.0),
                    ));
                    dot.setCornerRadius(3.0);
                    dot.setBackgroundColor(Some(&theme::cg(theme::ACCENT)));
                    layer.addSublayer(&dot);
                }
                view.addSubview(&accept);
            });
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

/// What the small card says above the replacement. The model's reason is
/// written for this; Harper's messages are written for a list ("Did you mean
/// to spell recieve this way?") and read oddly as a title, so local fixes get
/// a plain one.
fn title(fix: &Fix) -> String {
    if fix.source == Source::Ai && !fix.message.is_empty() {
        return fix.message.clone();
    }
    match fix.kind {
        Kind::Spelling => "Correct your spelling",
        Kind::Correctness => "Correct your grammar",
        Kind::Clarity => "Improve clarity",
    }
    .to_string()
}

fn card_view(width: f64, height: f64, mtm: MainThreadMarker) -> Retained<NSView> {
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height)),
    );
    view.setWantsLayer(true);
    still(|| {
        if let Some(layer) = view.layer() {
            surface(&layer, 12.0);
        }
    });
    view
}

const PAD: f64 = 16.0;

/// Title, the replacement as the thing to click, then Dismiss - and Add to
/// dictionary for a spelling. Nothing else: one word needs one decision.
fn word_card(
    index: usize,
    fix: &Fix,
    text: &str,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let tag = index as isize;
    let regular = unsafe { NSFontWeightRegular };
    let bold = unsafe { NSFontWeightSemibold };
    let heading = label(
        &[Run {
            text: &title(fix),
            color: theme::MUTED,
            size: 13.0,
            weight: regular,
            struck: false,
        }],
        mtm,
    );
    heading.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);

    let original = fix.original(text);
    let shown = if fix.replacement.is_empty() {
        format!("Remove “{original}”")
    } else {
        fix.replacement.clone()
    };
    let choice = flat(&shown, false, target, objc2::sel!(accept:), tag, mtm);
    choice.setAttributedTitle(&styled(&[Run {
        text: &shown,
        color: theme::ACCENT,
        size: 18.0,
        weight: bold,
        struck: false,
    }]));
    let dismiss = flat("Dismiss", false, target, objc2::sel!(dismiss:), tag, mtm);
    let learn = (fix.kind == Kind::Spelling).then(|| {
        flat(
            "Add to dictionary",
            false,
            target,
            objc2::sel!(learn:),
            tag,
            mtm,
        )
    });

    let width = (heading.fittingSize().width + PAD * 2.0)
        .max(choice.fittingSize().width + PAD * 2.0)
        .clamp(220.0, 340.0);
    let inner = width - PAD * 2.0;
    let rows: Vec<(&NSView, f64, f64)> = [
        Some((&*heading as &NSView, 16.0, 6.0)),
        Some((&*choice as &NSView, 26.0, 8.0)),
        Some((&*dismiss as &NSView, 24.0, 0.0)),
        learn.as_ref().map(|l| (&**l as &NSView, 24.0, 0.0)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let height = PAD * 2.0 + rows.iter().map(|(_, h, gap)| h + gap).sum::<f64>();
    let view = card_view(width, height, mtm);
    let mut top = height - PAD;
    for (row, h, gap) in rows {
        top -= h;
        // Buttons hug the left edge like the title above them; a centred
        // button title would sit indented from it.
        let x = if std::ptr::eq(row, &*heading as &NSView) {
            PAD
        } else {
            PAD - 4.0
        };
        row.setFrame(CGRect::new(CGPoint::new(x, top), CGSize::new(inner, h)));
        if let Some(button) = row.downcast_ref::<NSButton>() {
            button.setAlignment(objc2_app_kit::NSTextAlignment::Left);
        }
        view.addSubview(row);
        top -= gap;
    }
    view
}

/// The sentence with every change in place - struck in grey, the
/// replacement in bold - and one button that makes all of them.
fn sentence_card(
    index: usize,
    group: &[(usize, Fix)],
    text: &str,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let tag = index as isize;
    let regular = unsafe { NSFontWeightRegular };
    let bold = unsafe { NSFontWeightSemibold };
    let chars: Vec<char> = text.chars().collect();
    let first = group.first().map(|(_, f)| f.start).unwrap_or(0);
    let (from, to) = gramly_core::sentence(text, first);
    let to = group.iter().map(|(_, f)| f.end).max().unwrap_or(to).max(to);

    let mut pieces: Vec<(String, theme::Rgba, f64, bool)> = Vec::new();
    let mut cursor = from;
    for (_, fix) in group {
        if fix.start < cursor {
            continue;
        }
        pieces.push((
            chars[cursor..fix.start].iter().collect(),
            theme::TEXT,
            regular,
            false,
        ));
        let original: String = chars[fix.start..fix.end].iter().collect();
        if !original.is_empty() {
            pieces.push((original, theme::MUTED, regular, true));
            if !fix.replacement.is_empty() {
                pieces.push((" ".into(), theme::TEXT, regular, false));
            }
        }
        if !fix.replacement.is_empty() {
            pieces.push((fix.replacement.clone(), theme::TEXT, bold, false));
        }
        cursor = fix.end;
    }
    pieces.push((
        chars[cursor..to.min(chars.len())].iter().collect(),
        theme::TEXT,
        regular,
        false,
    ));
    let runs: Vec<Run> = pieces
        .iter()
        .map(|(t, color, weight, struck)| Run {
            text: t,
            color: *color,
            size: 15.0,
            weight: *weight,
            struck: *struck,
        })
        .collect();

    let width = 380.0;
    let inner = width - PAD * 2.0;
    let kind = group
        .iter()
        .map(|(_, f)| f.kind)
        .find(|k| *k != Kind::Clarity)
        .unwrap_or(Kind::Clarity);
    let heading = label(
        &[
            Run {
                text: "●  ",
                color: theme::kind(kind),
                size: 10.0,
                weight: regular,
                struck: false,
            },
            Run {
                text: "Correct the sentence",
                color: theme::MUTED,
                size: 13.0,
                weight: regular,
                struck: false,
            },
        ],
        mtm,
    );
    let body = label(&runs, mtm);
    body.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    body.setMaximumNumberOfLines(5);
    body.setPreferredMaxLayoutWidth(inner);
    let body_h = body.sizeThatFits(CGSize::new(inner, 400.0)).height.ceil();
    let correct = flat(
        "Correct the sentence",
        true,
        target,
        objc2::sel!(acceptSentence:),
        tag,
        mtm,
    );
    let dismiss = flat(
        "Dismiss",
        false,
        target,
        objc2::sel!(dismissSentence:),
        tag,
        mtm,
    );

    let row_h = 32.0;
    let height = PAD + 16.0 + 10.0 + body_h + 16.0 + row_h + PAD;
    let view = card_view(width, height, mtm);
    let mut top = height - PAD - 16.0;
    heading.setFrame(CGRect::new(
        CGPoint::new(PAD, top),
        CGSize::new(inner, 16.0),
    ));
    top -= 10.0 + body_h;
    body.setFrame(CGRect::new(
        CGPoint::new(PAD - 2.0, top),
        CGSize::new(inner + 4.0, body_h),
    ));
    let correct_w = correct.fittingSize().width + 28.0;
    correct.setFrame(CGRect::new(
        CGPoint::new(PAD, PAD),
        CGSize::new(correct_w, row_h),
    ));
    dismiss.setFrame(CGRect::new(
        CGPoint::new(PAD + correct_w + 10.0, PAD),
        CGSize::new(dismiss.fittingSize().width + 12.0, row_h),
    ));
    for sub in [&*heading as &NSView, &body, &correct, &dismiss] {
        view.addSubview(sub);
    }
    view
}
