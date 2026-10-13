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
    // The card and pill sit one level above the underlines, so a line from
    // a field behind never crosses the card.
    panel.setLevel(NSStatusWindowLevel + interactive as isize);
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

pub struct Overlay {
    mtm: MainThreadMarker,
    marks_panel: Retained<NSPanel>,
    drawn: Vec<Mark>,
    card: Retained<NSPanel>,
    /// The fix the card is showing.
    pub card_for: Option<usize>,
    /// The card's hoverable rows, and which one the pointer is over.
    rows: Vec<Row>,
    lit: Option<usize>,
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
            rows: Vec::new(),
            lit: None,
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
        let (view, rows) = if group.len() > 1 {
            sentence_card(index, group, text, target, self.mtm)
        } else {
            word_card(index, fix, text, target, self.mtm)
        };
        self.rows = rows;
        self.lit = None;
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
        self.rows.clear();
        self.lit = None;
    }

    /// Lights the row under the pointer. Driven by the same poll as the
    /// underlines: the card never becomes key, so it gets no tracking-area
    /// events of its own. The fade is short - long enough to read as a
    /// response, short enough that sweeping down the rows does not trail.
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
        CATransaction::setAnimationDuration(0.12);
        for (i, row) in self.rows.iter().enumerate() {
            if let Some(layer) = row.view.layer() {
                let color = if Some(i) == now { row.hover } else { row.rest };
                layer.setBackgroundColor(Some(&theme::cg(color)));
            }
        }
        CATransaction::commit();
        self.lit = now;
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

/// A row of the card that lights on hover and acts on click: a container
/// view whose layer is the highlight, its content, and a transparent button
/// over the whole of it, so the row is the target and not just its words.
struct Row {
    view: Retained<NSView>,
    rest: theme::Rgba,
    hover: theme::Rgba,
}

const CLEAR: theme::Rgba = theme::Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

fn row(
    frame: CGRect,
    rest: theme::Rgba,
    hover: theme::Rgba,
    action: Sel,
    tag: isize,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Row {
    let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
    view.setWantsLayer(true);
    still(|| {
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(8.0);
            layer.setBackgroundColor(Some(&theme::cg(rest)));
        }
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
    cover.setFrame(CGRect::new(CGPoint::new(0.0, 0.0), frame.size));
    // Added last by `finish`, so it sits above the row's content.
    Row { view, rest, hover }.with_cover(cover)
}

impl Row {
    fn with_cover(self, cover: Retained<NSButton>) -> Self {
        self.view.addSubview(&cover);
        self
    }

    /// Adds content under the cover button.
    fn add(&self, content: &NSView) {
        let cover = self.view.subviews().lastObject();
        match cover {
            Some(cover) => self.view.addSubview_positioned_relativeTo(
                content,
                objc2_app_kit::NSWindowOrderingMode::Below,
                Some(&cover),
            ),
            None => self.view.addSubview(content),
        }
    }
}

fn icon(symbol: &str, color: theme::Rgba, mtm: MainThreadMarker) -> Option<Retained<NSView>> {
    let image = objc2_app_kit::NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol),
        None,
    )?;
    image.setTemplate(true);
    let view = objc2_app_kit::NSImageView::imageViewWithImage(&image, mtm);
    view.setContentTintColor(Some(&theme::ns(color)));
    Some(Retained::into_super(Retained::into_super(view)))
}

/// What the small card says above the replacement. The model's titles are
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

/// Space between the card's edge and its rows.
const EDGE_PAD: f64 = 6.0;
/// Space inside a row; title, replacement and the action labels all start
/// at this inset so they share one left edge.
const ROW_PAD: f64 = 12.0;

/// The suggestion as one big target - title over the replacement - then a
/// Dismiss row and, for a spelling, an Add to dictionary row. The rows are
/// the whole interface: no buttons to aim at.
fn word_card(
    index: usize,
    fix: &Fix,
    text: &str,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> (Retained<NSView>, Vec<Row>) {
    let tag = index as isize;
    let regular = unsafe { NSFontWeightRegular };
    let medium = unsafe { NSFontWeightMedium };
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
    let choice = label(
        &[Run {
            text: &shown,
            color: theme::ACCENT,
            size: 18.0,
            weight: bold,
            struck: false,
        }],
        mtm,
    );
    choice.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);

    let mut actions: Vec<(&str, &str, Sel)> = vec![("trash", "Dismiss", objc2::sel!(dismiss:))];
    if fix.kind == Kind::Spelling {
        actions.push(("book.closed", "Add to dictionary", objc2::sel!(learn:)));
    }

    let content = heading
        .fittingSize()
        .width
        .max(choice.fittingSize().width)
        .max(150.0);
    let width = (content + (ROW_PAD + EDGE_PAD) * 2.0).clamp(240.0, 360.0);
    let row_w = width - EDGE_PAD * 2.0;
    let inner = row_w - ROW_PAD * 2.0;
    let main_h = 64.0;
    let action_h = 36.0;
    let height = EDGE_PAD * 2.0 + main_h + action_h * actions.len() as f64;
    let view = card_view(width, height, mtm);
    let mut rows = Vec::new();

    let mut top = height - EDGE_PAD - main_h;
    let main = row(
        CGRect::new(CGPoint::new(EDGE_PAD, top), CGSize::new(row_w, main_h)),
        CLEAR,
        theme::HOVER,
        objc2::sel!(accept:),
        tag,
        target,
        mtm,
    );
    heading.setFrame(CGRect::new(
        CGPoint::new(ROW_PAD, main_h - 12.0 - 16.0),
        CGSize::new(inner, 16.0),
    ));
    choice.setFrame(CGRect::new(
        CGPoint::new(ROW_PAD, 10.0),
        CGSize::new(inner, 24.0),
    ));
    main.add(&heading);
    main.add(&choice);
    view.addSubview(&main.view);
    rows.push(main);

    for (symbol, words, action) in actions {
        top -= action_h;
        let action_row = row(
            CGRect::new(CGPoint::new(EDGE_PAD, top), CGSize::new(row_w, action_h)),
            CLEAR,
            theme::HOVER,
            action,
            tag,
            target,
            mtm,
        );
        let mut x = ROW_PAD;
        if let Some(glyph) = icon(symbol, theme::MUTED, mtm) {
            glyph.setFrame(CGRect::new(
                CGPoint::new(x, (action_h - 16.0) / 2.0),
                CGSize::new(16.0, 16.0),
            ));
            action_row.add(&glyph);
            x += 26.0;
        }
        let caption = label(
            &[Run {
                text: words,
                color: theme::TEXT,
                size: 14.0,
                weight: medium,
                struck: false,
            }],
            mtm,
        );
        caption.setFrame(CGRect::new(
            CGPoint::new(x, (action_h - 18.0) / 2.0),
            CGSize::new(inner - x, 18.0),
        ));
        action_row.add(&caption);
        view.addSubview(&action_row.view);
        rows.push(action_row);
    }
    (view, rows)
}

/// The sentence with every change in place - struck in red, the
/// replacement in green - then "Correct the sentence" and Dismiss.
fn sentence_card(
    index: usize,
    group: &[(usize, Fix)],
    text: &str,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> (Retained<NSView>, Vec<Row>) {
    let tag = index as isize;
    let regular = unsafe { NSFontWeightRegular };
    let medium = unsafe { NSFontWeightMedium };
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
            pieces.push((original, theme::MISTAKE, regular, true));
            if !fix.replacement.is_empty() {
                pieces.push((" ".into(), theme::TEXT, regular, false));
            }
        }
        if !fix.replacement.is_empty() {
            pieces.push((fix.replacement.clone(), theme::ACCENT, bold, false));
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
    let pad = EDGE_PAD + ROW_PAD;
    let inner = width - pad * 2.0;
    let heading = label(
        &[Run {
            text: "Correct the sentence",
            color: theme::MUTED,
            size: 13.0,
            weight: regular,
            struck: false,
        }],
        mtm,
    );
    let body = label(&runs, mtm);
    body.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    body.setMaximumNumberOfLines(5);
    body.setPreferredMaxLayoutWidth(inner);
    let body_h = body.sizeThatFits(CGSize::new(inner, 400.0)).height.ceil();

    let button_h = 34.0;
    let height = pad + 16.0 + 10.0 + body_h + 16.0 + button_h + EDGE_PAD + 8.0;
    let view = card_view(width, height, mtm);
    let mut top = height - pad - 16.0;
    heading.setFrame(CGRect::new(
        CGPoint::new(pad, top),
        CGSize::new(inner, 16.0),
    ));
    top -= 10.0 + body_h;
    body.setFrame(CGRect::new(
        CGPoint::new(pad - 2.0, top),
        CGSize::new(inner + 4.0, body_h),
    ));
    view.addSubview(&heading);
    view.addSubview(&body);

    let mut rows = Vec::new();
    let y = EDGE_PAD + 8.0;
    let caption = |words: &str, color: theme::Rgba, weight: f64| {
        label(
            &[Run {
                text: words,
                color,
                size: 14.0,
                weight,
                struck: false,
            }],
            mtm,
        )
    };
    let correct_text = caption("Correct the sentence", theme::ON_ACCENT, bold);
    let correct_w = correct_text.fittingSize().width + 32.0;
    let correct = row(
        CGRect::new(CGPoint::new(pad, y), CGSize::new(correct_w, button_h)),
        theme::ACCENT,
        theme::ACCENT_HOVER,
        objc2::sel!(acceptSentence:),
        tag,
        target,
        mtm,
    );
    correct_text.setFrame(CGRect::new(
        CGPoint::new(16.0, (button_h - 18.0) / 2.0),
        CGSize::new(correct_w - 32.0, 18.0),
    ));
    correct.add(&correct_text);
    view.addSubview(&correct.view);
    rows.push(correct);

    let dismiss_text = caption("Dismiss", theme::TEXT, medium);
    let dismiss_w = dismiss_text.fittingSize().width + 28.0;
    let dismiss = row(
        CGRect::new(
            CGPoint::new(pad + correct_w + 8.0, y),
            CGSize::new(dismiss_w, button_h),
        ),
        CLEAR,
        theme::HOVER,
        objc2::sel!(dismissSentence:),
        tag,
        target,
        mtm,
    );
    dismiss_text.setFrame(CGRect::new(
        CGPoint::new(14.0, (button_h - 18.0) / 2.0),
        CGSize::new(dismiss_w - 28.0, 18.0),
    ));
    dismiss.add(&dismiss_text);
    view.addSubview(&dismiss.view);
    rows.push(dismiss);
    (view, rows)
}
