//! Ink: one neutral palette per appearance, ink for the main action, and the
//! underline's colour as the only colour Gramly draws. Nothing here belongs
//! to macOS - no system accent, no vibrancy, no SF font - so the Linux
//! surface can take the same values and look the same.
//!
//! It replaced Flow's palette, whose green carried every job (the fix, the
//! button, the glow after Accept) and whose dark card sat on white documents
//! too. The values match DESIGN.md.

use gramly_core::Kind;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_app_kit::{
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSBezierPath, NSColor, NSFont,
    NSFontWeightMedium, NSFontWeightRegular, NSFontWeightSemibold, NSImage, NSLineCapStyle,
};
use objc2_core_foundation::{CFData, CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColor;
use objc2_core_text::{CTFont, CTFontDescriptor, CTFontManagerCreateFontDescriptorFromData};
use objc2_foundation::NSArray;
use std::cell::RefCell;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

const fn hex(rgb: u32) -> Rgba {
    Rgba {
        r: ((rgb >> 16) & 0xFF) as f64 / 255.0,
        g: ((rgb >> 8) & 0xFF) as f64 / 255.0,
        b: (rgb & 0xFF) as f64 / 255.0,
        a: 1.0,
    }
}

const fn black(a: f64) -> Rgba {
    Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a,
    }
}

const fn white(a: f64) -> Rgba {
    Rgba {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a,
    }
}

pub const CLEAR: Rgba = black(0.0);

/// One appearance's colours.
pub struct Palette {
    /// Cards and the dictionary.
    pub surface: Rgba,
    /// The sentence, button labels.
    pub text: Rgba,
    /// Titles, struck words, hints.
    pub text2: Rgba,
    /// The outline of a secondary button and the field.
    pub line: Rgba,
    /// New words, and anything under the pointer.
    pub fill: Rgba,
    /// The main button.
    pub ink: Rgba,
    pub ink_hover: Rgba,
    pub on_ink: Rgba,
    /// The hairline around a card.
    pub edge: Rgba,
    /// How dark the card's soft shadow is.
    pub shadow: f32,
}

pub const LIGHT: Palette = Palette {
    surface: hex(0xFFFFFF),
    text: hex(0x18181B),
    text2: hex(0x66666E),
    line: black(0.13),
    fill: hex(0xF0F0F2),
    ink: hex(0x18181B),
    ink_hover: hex(0x303036),
    on_ink: hex(0xFFFFFF),
    edge: black(0.12),
    shadow: 0.2,
};

pub const DARK: Palette = Palette {
    surface: hex(0x1C1C1F),
    text: hex(0xEDEDEF),
    text2: hex(0xA1A1AA),
    line: white(0.13),
    fill: hex(0x2B2B30),
    ink: hex(0xEDEDEF),
    ink_hover: hex(0xFFFFFF),
    on_ink: hex(0x18181B),
    edge: white(0.1),
    shadow: 0.7,
};

/// Drawn over any app, light or dark, so the same in both appearances.
pub const MISTAKE: Rgba = hex(0xE5484D);
/// Clearer wording is advice, not an error: blue, as Grammarly draws it.
pub const ADVICE: Rgba = hex(0x4A7BF7);
/// The glow on a word just accepted: grey, so it reads on any app.
pub const ACCEPTED: Rgba = Rgba {
    r: 0.5,
    g: 0.5,
    b: 0.53,
    a: 0.34,
};

pub fn kind(kind: Kind) -> Rgba {
    match kind {
        Kind::Spelling | Kind::Correctness => MISTAKE,
        Kind::Clarity => ADVICE,
    }
}

/// The palette for the appearance the system is in now. Read whenever a
/// card or the dictionary is built, so a switch applies to the next one.
pub fn palette(mtm: MainThreadMarker) -> &'static Palette {
    if dark(mtm) { &DARK } else { &LIGHT }
}

pub fn dark(mtm: MainThreadMarker) -> bool {
    let names = unsafe { NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]) };
    NSApplication::sharedApplication(mtm)
        .effectiveAppearance()
        .bestMatchFromAppearancesWithNames(&names)
        .is_some_and(|name| &*name == unsafe { NSAppearanceNameDarkAqua })
}

pub fn ns(c: Rgba) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.r, c.g, c.b, c.a)
}

pub fn cg(c: Rgba) -> CFRetained<CGColor> {
    CGColor::new_srgb(c.r, c.g, c.b, c.a)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Medium,
    Semibold,
}

/// Atkinson Hyperlegible Next, bundled (SIL Open Font License, see
/// assets/fonts/OFL.txt). Its I, l and 1 never look alike, which matters
/// when the fix is a capital I, and a bundled font looks the same on every
/// machine, Linux included.
const FONTS: [(Weight, &[u8]); 3] = [
    (
        Weight::Regular,
        include_bytes!("../../assets/fonts/AtkinsonHyperlegibleNext-Regular.ttf"),
    ),
    (
        Weight::Medium,
        include_bytes!("../../assets/fonts/AtkinsonHyperlegibleNext-Medium.ttf"),
    ),
    (
        Weight::Semibold,
        include_bytes!("../../assets/fonts/AtkinsonHyperlegibleNext-SemiBold.ttf"),
    ),
];

thread_local! {
    /// Loaded from the bytes once per thread; fonts are only made on the
    /// main thread.
    static DESCRIPTORS: RefCell<Vec<(Weight, CFRetained<CTFontDescriptor>)>> =
        const { RefCell::new(Vec::new()) };
}

/// Gramly's font at `size`, or the system font if the bundled one failed to
/// load.
pub fn font(size: f64, weight: Weight) -> Retained<NSFont> {
    let descriptor = DESCRIPTORS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.is_empty() {
            for (w, bytes) in FONTS {
                let data = CFData::from_static_bytes(bytes);
                if let Some(d) = unsafe { CTFontManagerCreateFontDescriptorFromData(&data) } {
                    cache.push((w, d));
                }
            }
        }
        cache
            .iter()
            .find(|(w, _)| *w == weight)
            .map(|(_, d)| d.clone())
    });
    if let Some(descriptor) = descriptor {
        let font = unsafe { CTFont::with_font_descriptor(&descriptor, size, std::ptr::null()) };
        // CTFont and NSFont are toll-free bridged.
        let raw = CFRetained::into_raw(font).cast::<NSFont>();
        if let Some(font) = unsafe { Retained::from_raw(raw.as_ptr()) } {
            return font;
        }
    }
    let weight = unsafe {
        match weight {
            Weight::Regular => NSFontWeightRegular,
            Weight::Medium => NSFontWeightMedium,
            Weight::Semibold => NSFontWeightSemibold,
        }
    };
    NSFont::systemFontOfSize_weight(size, weight)
}

/// Gramly's mark: a g whose tail is the underline, drawn on a 20pt grid.
/// With no `tail` it is one colour and a template, for the menu bar; with
/// one, the bowl takes the label colour and the tail that colour.
pub fn mark(size: f64, tail: Option<Rgba>) -> Retained<NSImage> {
    let draw = block2::RcBlock::new(move |_rect: CGRect| -> Bool {
        let scale = size / 20.0;
        let at = |x: f64, y: f64| CGPoint::new(x * scale, y * scale);
        let stroke = |path: &NSBezierPath| {
            path.setLineWidth(if tail.is_none() { 1.9 } else { 2.0 } * scale);
            path.setLineCapStyle(NSLineCapStyle::Round);
            path.stroke();
        };
        let ink = if tail.is_some() {
            NSColor::labelColor()
        } else {
            NSColor::blackColor()
        };
        ink.setStroke();
        let r = 4.25;
        stroke(&NSBezierPath::bezierPathWithOvalInRect(CGRect::new(
            at(9.0 - r, 8.25 - r),
            CGSize::new(2.0 * r * scale, 2.0 * r * scale),
        )));
        let stem = NSBezierPath::bezierPath();
        stem.moveToPoint(at(13.25, 4.0));
        stem.lineToPoint(at(13.25, 13.75));
        stem.curveToPoint_controlPoint1_controlPoint2(
            at(10.0, 17.0),
            at(13.25, 15.55),
            at(11.8, 17.0),
        );
        stroke(&stem);
        if let Some(tail) = tail {
            ns(tail).setStroke();
        }
        let line = NSBezierPath::bezierPath();
        line.moveToPoint(at(10.0, 17.0));
        line.lineToPoint(at(3.5, 17.0));
        stroke(&line);
        Bool::YES
    });
    // Flipped, so the 20pt grid reads top-down like the SVG it came from.
    let image = NSImage::imageWithSize_flipped_drawingHandler(CGSize::new(size, size), true, &draw);
    image.setTemplate(tail.is_none());
    image
}

/// A cross for closing and removing, as a template so it takes the button's
/// tint.
pub fn cross(size: f64) -> Retained<NSImage> {
    let draw = block2::RcBlock::new(move |_rect: CGRect| -> Bool {
        let scale = size / 16.0;
        let path = NSBezierPath::bezierPath();
        path.moveToPoint(CGPoint::new(4.5 * scale, 4.5 * scale));
        path.lineToPoint(CGPoint::new(11.5 * scale, 11.5 * scale));
        path.moveToPoint(CGPoint::new(11.5 * scale, 4.5 * scale));
        path.lineToPoint(CGPoint::new(4.5 * scale, 11.5 * scale));
        path.setLineWidth(1.6 * scale);
        path.setLineCapStyle(NSLineCapStyle::Round);
        NSColor::blackColor().setStroke();
        path.stroke();
        Bool::YES
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(CGSize::new(size, size), true, &draw);
    image.setTemplate(true);
    image
}
