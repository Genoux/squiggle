//! Flow's palette, so the two tools read as one family: near-black ground,
//! quiet raised surfaces, off-white text, green only for what is active.
//! Values are copied from Flow's `crates/console/src/theme.rs`.

use gramly_core::Kind;
use objc2::rc::Retained;
use objc2_app_kit::NSColor;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGColor;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

const fn rgb(r: f64, g: f64, b: f64) -> Rgba {
    Rgba { r, g, b, a: 1.0 }
}

/// Flow `RAISED` #161719.
pub const SURFACE: Rgba = rgb(0.086, 0.090, 0.098);
/// Flow `EDGE` #282A2C.
pub const EDGE: Rgba = rgb(0.157, 0.165, 0.173);
/// Flow `FG` #ECEDEF.
pub const TEXT: Rgba = rgb(0.925, 0.929, 0.937);
/// Flow `MUTED` #8C9094.
pub const MUTED: Rgba = rgb(0.549, 0.565, 0.580);
/// Flow `ACCENT` #2ED573 - Accept, and nothing else.
pub const ACCENT: Rgba = rgb(0.180, 0.835, 0.451);
/// Flow `ON_ACCENT` #14150F - text on the green Accept button.
pub const ON_ACCENT: Rgba = rgb(0.078, 0.082, 0.059);
/// Flow `ERR` #D4736B, a touch more saturated so a 2px line still reads on
/// white documents as well as dark ones.
pub const MISTAKE: Rgba = rgb(0.890, 0.373, 0.337);
/// Clarity is advice, not an error: blue, as Grammarly draws it.
pub const ADVICE: Rgba = rgb(0.278, 0.553, 0.965);

pub fn kind(kind: Kind) -> Rgba {
    match kind {
        Kind::Spelling | Kind::Correctness => MISTAKE,
        Kind::Clarity => ADVICE,
    }
}

pub fn ns(c: Rgba) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.r, c.g, c.b, c.a)
}

pub fn cg(c: Rgba) -> CFRetained<CGColor> {
    CGColor::new_srgb(c.r, c.g, c.b, c.a)
}
