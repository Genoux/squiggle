//! Ink: one neutral palette per appearance, ink for the main action, and
//! colour only where it means something - the underline, the words a fix
//! takes out (red) and puts in (green). Nothing here belongs to macOS - no
//! system accent, no vibrancy, no SF font - so the Linux surface can take
//! the same values and look the same.
//!
//! It replaced Flow's palette, whose green carried every job (the fix, the
//! button, the glow after Accept) and whose dark card sat on white documents
//! too. The values are the ones in design/tokens.json; a test holds them
//! together.

use gramly_core::Kind;

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

const fn hexa(rgb: u32, a: f64) -> Rgba {
    let c = hex(rgb);
    Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a,
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
    /// Titles, hints.
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
    /// A word a correction takes out: struck, in red, line and letters
    /// alike.
    pub gone: Rgba,
    /// A word clearer wording takes out, in blue.
    pub gone_advice: Rgba,
    /// A word a fix puts in: green, bold, on a green fill.
    pub new: Rgba,
    pub new_fill: Rgba,
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
    gone: hex(0xCE2C31),
    gone_advice: hex(0x2F5BD3),
    new: hex(0x18794E),
    new_fill: hexa(0x30A46C, 0.15),
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
    // Coral, GitHub's deleted code in its dark theme. A pale salmon
    // (#FF9592) passed on contrast but read as pink, and under its darker
    // line as two reds over each other.
    gone: hex(0xFF7B72),
    gone_advice: hex(0x9DB5FF),
    new: hex(0x3DD68C),
    new_fill: hexa(0x30A46C, 0.22),
};

/// Drawn over any app, light or dark, so the same in both appearances.
pub const MISTAKE: Rgba = hex(0xE5484D);
/// Clearer wording is advice, not an error: blue, as Grammarly draws it.
pub const ADVICE: Rgba = hex(0x4A7BF7);
/// The glow on a word just accepted: the fix's green, faint enough to read
/// on any app.
pub const ACCEPTED: Rgba = hexa(0x30A46C, 0.34);

/// Corners. The card and the dictionary window; buttons and the field; the
/// fill behind a new word and the glow on a word. 12, 8 and 5 read as
/// bubbly.
pub const CORNER: f64 = 8.0;
pub const CORNER_CONTROL: f64 = 6.0;
pub const CORNER_INLINE: f64 = 3.0;

pub fn kind(kind: Kind) -> Rgba {
    match kind {
        Kind::Spelling | Kind::Correctness => MISTAKE,
        Kind::Clarity => ADVICE,
    }
}

#[cfg(target_os = "linux")]
fn byte(channel: f64) -> u8 {
    (channel * 255.0).round() as u8
}

#[cfg(target_os = "linux")]
impl Rgba {
    /// For Pango markup, which has no `rgba()`.
    pub fn hex(self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}",
            byte(self.r),
            byte(self.g),
            byte(self.b)
        )
    }

    /// For a stylesheet.
    pub fn css(self) -> String {
        format!(
            "rgba({}, {}, {}, {})",
            byte(self.r),
            byte(self.g),
            byte(self.b),
            self.a
        )
    }
}

/// design/tokens.json is the design system's copy of these values, for the
/// page and the Linux surface. These tests fail when the two disagree.
#[cfg(test)]
pub(crate) mod tokens {
    use super::*;
    use serde_json::Value;

    pub(crate) fn load() -> Value {
        serde_json::from_str(include_str!("../../design/tokens.json"))
            .expect("design/tokens.json parses")
    }

    fn at<'a>(tokens: &'a Value, path: &str) -> &'a Value {
        path.split('.').fold(tokens, |v, key| &v[key])
    }

    pub(crate) fn num(tokens: &Value, path: &str) -> f64 {
        at(tokens, path)
            .as_f64()
            .unwrap_or_else(|| panic!("tokens.json has no number at {path}"))
    }

    fn color(tokens: &Value, path: &str) -> Rgba {
        let text = at(tokens, path)
            .as_str()
            .unwrap_or_else(|| panic!("tokens.json has no colour at {path}"));
        let digits = text.strip_prefix('#').expect("a colour starts with #");
        let byte = |i: usize| f64::from(u8::from_str_radix(&digits[i..i + 2], 16).unwrap()) / 255.0;
        Rgba {
            r: byte(0),
            g: byte(2),
            b: byte(4),
            a: if digits.len() == 8 { byte(6) } else { 1.0 },
        }
    }

    /// Equal as #RRGGBBAA writes them: the channels exactly, the alpha to
    /// the nearest step.
    fn same(a: Rgba, b: Rgba) -> bool {
        let exact = |x: f64, y: f64| (x - y).abs() < 1e-9;
        exact(a.r, b.r)
            && exact(a.g, b.g)
            && exact(a.b, b.b)
            && (a.a - b.a).abs() <= 0.5 / 255.0 + 1e-6
    }

    fn fields(p: &Palette) -> [(&'static str, Rgba); 13] {
        [
            ("surface", p.surface),
            ("text", p.text),
            ("text2", p.text2),
            ("line", p.line),
            ("fill", p.fill),
            ("ink", p.ink),
            ("ink_hover", p.ink_hover),
            ("on_ink", p.on_ink),
            ("edge", p.edge),
            ("gone", p.gone),
            ("gone_advice", p.gone_advice),
            ("new", p.new),
            ("new_fill", p.new_fill),
        ]
    }

    #[test]
    fn palettes_match_the_tokens() {
        let t = load();
        for (mode, p) in [("light", &LIGHT), ("dark", &DARK)] {
            let listed = t["color"][mode].as_object().expect("a palette").len();
            assert_eq!(listed, fields(p).len(), "{mode}: a colour on one side only");
            for (name, ours) in fields(p) {
                let theirs = color(&t, &format!("color.{mode}.{name}"));
                assert!(
                    same(ours, theirs),
                    "{mode}.{name}: {ours:?} here, {theirs:?} in tokens.json"
                );
            }
            let strength = num(&t, &format!("shadow.strength.{mode}"));
            assert!(
                (f64::from(p.shadow) - strength).abs() < 1e-6,
                "{mode} shadow"
            );
        }
        for (name, ours) in [
            ("mistake", MISTAKE),
            ("advice", ADVICE),
            ("accepted", ACCEPTED),
        ] {
            let theirs = color(&t, &format!("color.both.{name}"));
            assert!(
                same(ours, theirs),
                "both.{name}: {ours:?} here, {theirs:?} in tokens.json"
            );
        }
    }

    #[test]
    fn corners_match_the_tokens() {
        let t = load();
        assert_eq!(num(&t, "corner.surface"), CORNER);
        assert_eq!(num(&t, "corner.control"), CORNER_CONTROL);
        assert_eq!(num(&t, "corner.inline"), CORNER_INLINE);
    }

    #[test]
    fn the_glow_lasts_as_long_as_the_tokens_say() {
        let t = load();
        let glow = crate::surface::engine::FLASH.as_millis() as f64;
        assert_eq!(num(&t, "motion.accepted_ms"), glow);
    }

    /// The page writes each colour as it is in the file, so one changed
    /// here and not there is caught.
    #[test]
    fn the_page_shows_every_colour() {
        let t = load();
        let page = include_str!("../../design/index.html").to_uppercase();
        for group in ["light", "dark", "both"] {
            for (name, value) in t["color"][group].as_object().expect("a group") {
                let hex = value.as_str().expect("a colour").to_uppercase();
                assert!(
                    page.contains(&hex),
                    "design/index.html lacks {group}.{name} {hex}"
                );
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;

    #[test]
    fn css_rounds_channels_to_bytes() {
        assert_eq!(MISTAKE.css(), "rgba(229, 72, 77, 1)");
        assert_eq!(MISTAKE.hex(), "#e5484d");
        assert_eq!(Rgba { a: 0.2, ..MISTAKE }.css(), "rgba(229, 72, 77, 0.2)");
    }
}
