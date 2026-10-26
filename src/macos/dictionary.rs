//! The dictionary window: the words Gramly never flags, a field to add one,
//! the list to remove one from. Opened from the menu, where it replaced
//! opening `dictionary.txt` in TextEdit.
//!
//! Drawn in Gramly's own look, not the Mac's: a borderless window with its
//! own header and close button, the card's surface, buttons and spacing. A
//! titled window brought the traffic lights and the system's title bar,
//! which the Linux surface will not have. Borderless windows refuse keys
//! unless they say otherwise, so `KeyWindow` does; Escape and Command-W
//! close it.
//!
//! The file stays the store, so a word added from a card, or typed into the
//! file by hand, shows the next time the window opens. The content is built
//! again each time it opens, so it follows a change of appearance.

use crate::macos::overlay::plain_button;
use crate::macos::theme::{self, Face, Palette, Rgba};
use crate::surface::dictionary;
use gramly_core::config;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSBorderType, NSButton, NSColor, NSFocusRingType,
    NSFontAttributeName, NSForegroundColorAttributeName, NSLineBreakMode, NSResponder,
    NSScrollView, NSTextAlignment, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSDictionary, NSObject, NSString,
};

const WIDTH: f64 = theme::DICTIONARY_WIDTH;
const HEIGHT: f64 = theme::DICTIONARY_HEIGHT;
const PAD: f64 = theme::DICTIONARY_PAD;
const ROW: f64 = theme::DICTIONARY_ROW;
/// Where the field sits, and where the list starts and stops, from the top
/// and bottom of the window.
const FIELD_TOP: f64 = 74.0;
const LIST_TOP: f64 = FIELD_TOP + 32.0 + 12.0;
const LIST_BOTTOM: f64 = 40.0;

define_class!(
    #[unsafe(super(NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GramlyKeyWindow"]
    struct KeyWindow;

    unsafe impl NSObjectProtocol for KeyWindow {}

    impl KeyWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        /// Command-W, and the close button: a borderless window has no
        /// close button for `performClose:` to press.
        #[unsafe(method(performClose:))]
        fn perform_close(&self, _sender: Option<&AnyObject>) {
            self.orderOut(None);
        }

        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: Option<&AnyObject>) {
            self.orderOut(None);
        }
    }
);

impl KeyWindow {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe {
            msg_send![
                super(this),
                initWithContentRect: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(WIDTH, HEIGHT)),
                styleMask: NSWindowStyleMask::Borderless,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        }
    }
}

pub struct Dictionary {
    window: Retained<KeyWindow>,
    field: Retained<NSTextField>,
    list: Retained<NSScrollView>,
    count: Retained<NSTextField>,
    /// As shown, sorted; a remove button's tag is its index here.
    words: Vec<String>,
}

fn label(text: &str, face: Face, color: Rgba, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&theme::font(face.size, face.weight)));
    label.setTextColor(Some(&theme::ns(color)));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label
}

/// A frame `top` points below the top of a view `height` tall: AppKit
/// counts from the bottom.
fn from_top(x: f64, top: f64, width: f64, height: f64, within: f64) -> CGRect {
    CGRect::new(
        CGPoint::new(x, within - top - height),
        CGSize::new(width, height),
    )
}

/// A borderless button showing the cross, tinted.
fn cross_button(
    target: &AnyObject,
    action: objc2::runtime::Sel,
    color: Rgba,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithImage_target_action(
            &theme::cross(14.0),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setBordered(false);
    button.setContentTintColor(Some(&theme::ns(color)));
    button
}

impl Dictionary {
    pub fn new(target: &AnyObject, mtm: MainThreadMarker) -> Self {
        let window = KeyWindow::new(mtm);
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setHasShadow(true);
        window.setMovableByWindowBackground(true);
        unsafe { window.setReleasedWhenClosed(false) };
        let (field, list, count) = Self::build(&window, target, mtm);
        Self {
            window,
            field,
            list,
            count,
            words: Vec::new(),
        }
    }

    /// The window's content in the current appearance.
    fn build(
        window: &KeyWindow,
        target: &AnyObject,
        mtm: MainThreadMarker,
    ) -> (
        Retained<NSTextField>,
        Retained<NSScrollView>,
        Retained<NSTextField>,
    ) {
        let p = theme::palette(mtm);
        let content = NSView::initWithFrame(
            NSView::alloc(mtm),
            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(WIDTH, HEIGHT)),
        );
        content.setWantsLayer(true);
        if let Some(layer) = content.layer() {
            layer.setBackgroundColor(Some(&theme::cg(p.surface)));
            layer.setCornerRadius(theme::CORNER);
            layer.setBorderWidth(0.5);
            layer.setBorderColor(Some(&theme::cg(p.edge)));
        }
        let inner = WIDTH - PAD * 2.0;

        // Header: the title and the close button. No logo until the name
        // is settled.
        let title = label(dictionary::TITLE, theme::DICTIONARY_TITLE, p.text, mtm);
        let title_h = title.fittingSize().height;
        title.setFrame(from_top(
            PAD,
            PAD + ((20.0 - title_h) / 2.0).round(),
            inner - 32.0,
            title_h,
            HEIGHT,
        ));
        content.addSubview(&title);
        let close = cross_button(window, sel!(performClose:), p.text2, mtm);
        close.setToolTip(Some(&NSString::from_str("Close")));
        close.setFrame(from_top(
            WIDTH - PAD - 28.0 + 6.0,
            PAD - 4.0,
            28.0,
            28.0,
            HEIGHT,
        ));
        content.addSubview(&close);

        let sub = label(dictionary::SUBTITLE, theme::DICTIONARY_HINT, p.text2, mtm);
        sub.setFrame(from_top(PAD, PAD + 24.0, inner, 18.0, HEIGHT));
        content.addSubview(&sub);

        // The field and its button.
        let add = plain_button(dictionary::ADD, false, sel!(addWord:), target, p, mtm);
        let add_w = add.frame().size.width;
        add.setFrameOrigin(from_top(WIDTH - PAD - add_w, FIELD_TOP, add_w, 32.0, HEIGHT).origin);
        content.addSubview(&add);
        let box_w = inner - add_w - 8.0;
        let frame = NSView::initWithFrame(
            NSView::alloc(mtm),
            from_top(PAD, FIELD_TOP, box_w, 32.0, HEIGHT),
        );
        frame.setWantsLayer(true);
        if let Some(layer) = frame.layer() {
            layer.setCornerRadius(theme::CORNER_CONTROL);
            layer.setBorderWidth(1.0);
            layer.setBorderColor(Some(&theme::cg(p.line)));
        }
        content.addSubview(&frame);
        let font = theme::font(theme::DICTIONARY_WORD.size, theme::DICTIONARY_WORD.weight);
        let field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(box_w - 20.0, 20.0)),
        );
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFont(Some(&font));
        field.setTextColor(Some(&theme::ns(p.text)));
        // The ring would be the system accent; the outline and the caret
        // already show where typing goes.
        field.setFocusRingType(NSFocusRingType::None);
        let placeholder = {
            let color = theme::ns(p.text2);
            let keys: [&NSAttributedStringKey; 2] =
                unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
            let values: [&AnyObject; 2] = [&font, &color];
            let attributes = NSDictionary::from_slices(&keys, &values);
            unsafe {
                NSAttributedString::initWithString_attributes(
                    NSAttributedString::alloc(),
                    &NSString::from_str(dictionary::PLACEHOLDER),
                    Some(&attributes),
                )
            }
        };
        field.setPlaceholderAttributedString(Some(&placeholder));
        let field_h = field.fittingSize().height.ceil();
        field.setFrame(CGRect::new(
            CGPoint::new(10.0, ((32.0 - field_h) / 2.0).round()),
            CGSize::new(box_w - 20.0, field_h),
        ));
        // Return in the field adds, the same as the button.
        unsafe {
            field.setTarget(Some(target));
            field.setAction(Some(sel!(addWord:)));
        }
        frame.addSubview(&field);

        let list = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            CGRect::new(
                CGPoint::new(PAD - 8.0, LIST_BOTTOM),
                CGSize::new(inner + 16.0, HEIGHT - LIST_TOP - LIST_BOTTOM),
            ),
        );
        list.setHasVerticalScroller(true);
        list.setAutohidesScrollers(true);
        list.setDrawsBackground(false);
        list.setBorderType(NSBorderType::NoBorder);
        content.addSubview(&list);

        let count = label("", theme::DICTIONARY_COUNT, p.text2, mtm);
        count.setFrame(CGRect::new(
            CGPoint::new(PAD, 14.0),
            CGSize::new(inner, 16.0),
        ));
        content.addSubview(&count);

        window.setContentView(Some(&content));
        (field, list, count)
    }

    /// Reads the file again and brings the window forward, ready to type.
    pub fn show(&mut self, target: &AnyObject, mtm: MainThreadMarker) {
        let first = !self.window.isVisible();
        if first {
            let (field, list, count) = Self::build(&self.window, target, mtm);
            (self.field, self.list, self.count) = (field, list, count);
        }
        self.reload(target, mtm);
        if first {
            self.window.center();
        }
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        self.window.makeKeyAndOrderFront(None);
        self.window.makeFirstResponder(Some(&self.field));
    }

    pub fn visible(&self) -> bool {
        self.window.isVisible()
    }

    pub fn reload(&mut self, target: &AnyObject, mtm: MainThreadMarker) {
        self.words = config::sorted_dictionary();
        self.render(target, mtm);
    }

    /// Shows `words` without reading the file, for `gramly preview`.
    pub(crate) fn preview(&mut self, words: &[&str], target: &AnyObject, mtm: MainThreadMarker) {
        self.words = words.iter().map(|w| w.to_string()).collect();
        self.render(target, mtm);
    }

    pub(crate) fn content(&self) -> Option<Retained<NSView>> {
        self.window.contentView()
    }

    /// Adds what the field holds. Returns whether the dictionary changed.
    pub fn add(&mut self, target: &AnyObject, mtm: MainThreadMarker) -> bool {
        let input = self.field.stringValue().to_string();
        self.field.setStringValue(&NSString::from_str(""));
        let added = config::learn_word(&input).unwrap_or_else(|err| {
            note!("adding {input:?} to the dictionary: {err:#}");
            false
        });
        self.reload(target, mtm);
        added
    }

    /// Removes the word shown at `index`. Returns whether it was removed.
    pub fn remove(&mut self, index: usize, target: &AnyObject, mtm: MainThreadMarker) -> bool {
        if index >= self.words.len() {
            return false;
        }
        let word = &self.words[index];
        if let Err(err) = config::forget_word(word) {
            note!("removing {word:?} from the dictionary: {err:#}");
        }
        self.reload(target, mtm);
        true
    }

    fn render(&self, target: &AnyObject, mtm: MainThreadMarker) {
        let p: &Palette = theme::palette(mtm);
        let clip = self.list.contentSize();
        let width = clip.width;
        let height = (self.words.len() as f64 * ROW).max(clip.height);
        let document = NSView::initWithFrame(
            NSView::alloc(mtm),
            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height)),
        );
        if self.words.is_empty() {
            let empty = label(dictionary::EMPTY, theme::DICTIONARY_HINT, p.text2, mtm);
            empty.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
            empty.setAlignment(NSTextAlignment::Center);
            empty.setFrame(from_top(24.0, 28.0, width - 48.0, 40.0, height));
            document.addSubview(&empty);
        }
        for (index, word) in self.words.iter().enumerate() {
            let top = index as f64 * ROW;
            let text = label(word, theme::DICTIONARY_WORD, p.text, mtm);
            let text_h = text.fittingSize().height;
            text.setFrame(from_top(
                8.0,
                top + ((ROW - text_h) / 2.0).round(),
                width - 52.0,
                text_h,
                height,
            ));
            document.addSubview(&text);
            let remove = cross_button(target, sel!(removeWord:), p.text2, mtm);
            remove.setTag(index as isize);
            remove.setToolTip(Some(&NSString::from_str(&format!("Remove {word}"))));
            remove.setFrame(from_top(width - 32.0, top + 4.0, 28.0, 28.0, height));
            document.addSubview(&remove);
        }
        self.list.setDocumentView(Some(&document));
        document.scrollPoint(CGPoint::new(0.0, height));
        self.count
            .setStringValue(&NSString::from_str(&dictionary::count(self.words.len())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_match_the_tokens() {
        use crate::macos::theme::tokens::{load, num};
        let t = load();
        assert_eq!(num(&t, "dictionary.width"), WIDTH);
        assert_eq!(num(&t, "dictionary.height"), HEIGHT);
        assert_eq!(num(&t, "dictionary.padding"), PAD);
        assert_eq!(num(&t, "dictionary.row"), ROW);
    }
}
