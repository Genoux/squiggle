//! The dictionary window: the words Gramly never flags, a field to add one,
//! the list to remove one from. Opened from the menu, where it replaced
//! opening `dictionary.txt` in TextEdit.
//!
//! Neutral dark greys rather than Flow's palette: the window is a utility,
//! and it should read as part of the Mac before it reads as part of a brand.
//! The file stays the store, so a word added from a card, or typed into the
//! file by hand, shows the next time the window opens.

use crate::macos::theme::{self, Rgba};
use gramly_core::config;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSApplication,
    NSBackingStoreType, NSBorderType, NSButton, NSFont, NSFontWeightRegular, NSFontWeightSemibold,
    NSImage, NSLineBreakMode, NSScrollView, NSTextField, NSTextFieldBezelStyle, NSView, NSWindow,
    NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::NSString;

const BACKGROUND: Rgba = Rgba {
    r: 0.118,
    g: 0.118,
    b: 0.118,
    a: 1.0,
}; // #1E1E1E
const TEXT: Rgba = Rgba {
    r: 0.961,
    g: 0.961,
    b: 0.961,
    a: 1.0,
}; // #F5F5F5
const MUTED: Rgba = Rgba {
    r: 0.604,
    g: 0.604,
    b: 0.604,
    a: 1.0,
}; // #9A9A9A

const WIDTH: f64 = 340.0;
const HEIGHT: f64 = 440.0;
const PAD: f64 = 16.0;
const ROW: f64 = 30.0;
/// Where the list starts and stops, from the top and bottom of the window.
const LIST_TOP: f64 = 132.0;
const LIST_BOTTOM: f64 = 34.0;

pub struct Dictionary {
    window: Retained<NSWindow>,
    field: Retained<NSTextField>,
    list: Retained<NSScrollView>,
    count: Retained<NSTextField>,
    /// As shown, sorted; a remove button's tag is its index here.
    words: Vec<String>,
}

fn label(
    text: &str,
    size: f64,
    bold: bool,
    color: Rgba,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    let weight = unsafe {
        if bold {
            NSFontWeightSemibold
        } else {
            NSFontWeightRegular
        }
    };
    label.setFont(Some(&NSFont::systemFontOfSize_weight(size, weight)));
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

impl Dictionary {
    pub fn new(target: &AnyObject, mtm: MainThreadMarker) -> Self {
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(WIDTH, HEIGHT)),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::FullSizeContentView,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str("Dictionary"));
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        window.setTitlebarAppearsTransparent(true);
        window.setMovableByWindowBackground(true);
        window.setBackgroundColor(Some(&theme::ns(BACKGROUND)));
        unsafe { window.setReleasedWhenClosed(false) };
        if let Some(dark) = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }) {
            window.setAppearance(Some(&dark));
        }

        let content = NSView::initWithFrame(
            NSView::alloc(mtm),
            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(WIDTH, HEIGHT)),
        );
        let inner = WIDTH - PAD * 2.0;

        let title = label("Dictionary", 17.0, true, TEXT, mtm);
        title.setFrame(from_top(PAD, 40.0, inner, 22.0, HEIGHT));
        content.addSubview(&title);

        let sub = label("Words Gramly never flags.", 12.5, false, MUTED, mtm);
        sub.setFrame(from_top(PAD, 64.0, inner, 18.0, HEIGHT));
        content.addSubview(&sub);

        let add_width = 64.0;
        let field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            from_top(PAD, 94.0, inner - add_width - 8.0, 28.0, HEIGHT),
        );
        field.setBezeled(true);
        field.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
        field.setPlaceholderString(Some(&NSString::from_str("Add a word")));
        field.setFont(Some(&NSFont::systemFontOfSize(14.0)));
        // The ring is the system accent, usually blue: the one colour in a
        // window meant to stay neutral. The caret already shows focus.
        field.setFocusRingType(objc2_app_kit::NSFocusRingType::None);
        // Return in the field adds, the same as the button.
        unsafe {
            field.setTarget(Some(target));
            field.setAction(Some(sel!(addWord:)));
        }
        content.addSubview(&field);

        let add = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Add"),
                Some(target),
                Some(sel!(addWord:)),
                mtm,
            )
        };
        add.setFrame(from_top(
            WIDTH - PAD - add_width,
            94.0,
            add_width,
            28.0,
            HEIGHT,
        ));
        content.addSubview(&add);

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

        let count = label("", 12.0, false, MUTED, mtm);
        count.setFrame(CGRect::new(
            CGPoint::new(PAD, 10.0),
            CGSize::new(inner, 16.0),
        ));
        content.addSubview(&count);

        window.setContentView(Some(&content));
        Self {
            window,
            field,
            list,
            count,
            words: Vec::new(),
        }
    }

    /// Reads the file again and brings the window forward, ready to type.
    pub fn show(&mut self, target: &AnyObject, mtm: MainThreadMarker) {
        let first = !self.window.isVisible();
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
        let mut words = config::dictionary();
        words.sort_by_key(|w| w.to_lowercase());
        words.dedup_by_key(|w| w.to_lowercase());
        self.words = words;
        self.render(target, mtm);
    }

    /// Adds what the field holds. Returns whether the dictionary changed.
    pub fn add(&mut self, target: &AnyObject, mtm: MainThreadMarker) -> bool {
        let word = self.field.stringValue().to_string().trim().to_string();
        self.field.setStringValue(&NSString::from_str(""));
        if word.is_empty() || word.contains(char::is_whitespace) {
            return false;
        }
        let known = self
            .words
            .iter()
            .any(|w| w.to_lowercase() == word.to_lowercase());
        if !known && let Err(err) = config::add_to_dictionary(&word) {
            note!("adding {word:?} to the dictionary: {err:#}");
            return false;
        }
        self.reload(target, mtm);
        !known
    }

    /// Removes the word shown at `index`. Returns whether it was removed.
    pub fn remove(&mut self, index: usize, target: &AnyObject, mtm: MainThreadMarker) -> bool {
        if index >= self.words.len() {
            return false;
        }
        let word = self.words.remove(index);
        if let Err(err) = config::save_dictionary(&self.words) {
            note!("removing {word:?} from the dictionary: {err:#}");
        }
        self.reload(target, mtm);
        true
    }

    fn render(&self, target: &AnyObject, mtm: MainThreadMarker) {
        let clip = self.list.contentSize();
        let width = clip.width;
        let height = (self.words.len() as f64 * ROW).max(clip.height);
        let document = NSView::initWithFrame(
            NSView::alloc(mtm),
            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height)),
        );
        if self.words.is_empty() {
            let empty = label(
                "No words yet. Add one above, or choose\n\u{201c}Add to dictionary\u{201d} on a suggestion.",
                12.5,
                false,
                MUTED,
                mtm,
            );
            empty.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
            empty.setAlignment(objc2_app_kit::NSTextAlignment::Center);
            empty.setFrame(from_top(0.0, 40.0, width, 40.0, height));
            document.addSubview(&empty);
        }
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("xmark"),
            Some(&NSString::from_str("Remove")),
        );
        for (index, word) in self.words.iter().enumerate() {
            let top = index as f64 * ROW;
            let text = label(word, 14.0, false, TEXT, mtm);
            text.setFrame(from_top(8.0, top + 6.0, width - 50.0, 18.0, height));
            document.addSubview(&text);
            if let Some(image) = &image {
                let remove = unsafe {
                    NSButton::buttonWithImage_target_action(
                        image,
                        Some(target),
                        Some(sel!(removeWord:)),
                        mtm,
                    )
                };
                remove.setBordered(false);
                remove.setContentTintColor(Some(&theme::ns(MUTED)));
                remove.setTag(index as isize);
                remove.setToolTip(Some(&NSString::from_str(&format!("Remove {word}"))));
                remove.setFrame(from_top(width - 34.0, top + 5.0, 22.0, 20.0, height));
                document.addSubview(&remove);
            }
        }
        self.list.setDocumentView(Some(&document));
        document.scrollPoint(CGPoint::new(0.0, height));
        let n = self.words.len();
        self.count.setStringValue(&NSString::from_str(&match n {
            0 => String::new(),
            1 => "1 word".into(),
            n => format!("{n} words"),
        }));
    }
}
