//! The AppKit half of the palette: appearance, colours, the bundled font.
//! The values are in `crate::surface::theme`, shared with Linux.

pub use crate::surface::theme::*;
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

/// The palette for the appearance the system is in now. Read whenever a
/// card or the dictionary is built, so a switch applies to the next one.
pub fn palette(mtm: MainThreadMarker) -> &'static Palette {
    palette_for(dark(mtm))
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

thread_local! {
    /// Loaded from the bytes once per thread; fonts are only made on the
    /// main thread.
    static DESCRIPTORS: RefCell<Vec<(Weight, CFRetained<CTFontDescriptor>)>> =
        const { RefCell::new(Vec::new()) };
}

/// Squiggle's font at `size`, or the system font if the bundled one failed to
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

/// The icon of one of the ⋯'s parts, `size` square, as a template so it
/// takes the tint of whatever shows it; see `surface::theme::icon`.
pub fn icon_image(action: crate::surface::Action, size: f64) -> Retained<NSImage> {
    let shapes = icon(action);
    let draw = block2::RcBlock::new(move |_rect: CGRect| -> Bool {
        let s = size / 16.0;
        NSColor::blackColor().setStroke();
        NSColor::blackColor().setFill();
        for shape in shapes {
            let oval = |x: f64, y: f64, r: f64| {
                NSBezierPath::bezierPathWithOvalInRect(CGRect::new(
                    CGPoint::new((x - r) * s, (y - r) * s),
                    CGSize::new(r * 2.0 * s, r * 2.0 * s),
                ))
            };
            let path = match *shape {
                Shape::Dot { x, y, r } => {
                    oval(x, y, r).fill();
                    continue;
                }
                Shape::Ring { x, y, r } => oval(x, y, r),
                Shape::Line { from, to } => {
                    let path = NSBezierPath::bezierPath();
                    path.moveToPoint(CGPoint::new(from.0 * s, from.1 * s));
                    path.lineToPoint(CGPoint::new(to.0 * s, to.1 * s));
                    path
                }
                Shape::Bar { x, y, w, h, r } => {
                    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                        CGRect::new(CGPoint::new(x * s, y * s), CGSize::new(w * s, h * s)),
                        r * s,
                        r * s,
                    )
                }
            };
            path.setLineWidth(ICON_STROKE * s);
            path.setLineCapStyle(NSLineCapStyle::Round);
            path.stroke();
        }
        Bool::YES
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(CGSize::new(size, size), true, &draw);
    image.setTemplate(true);
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
