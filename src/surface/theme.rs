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

use super::Mark;
use gramly_core::Kind;
use std::time::Duration;

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

pub fn palette_for(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}

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

/// What a card strikes out: clearer wording is advice, blue, and anything
/// else a mistake, red.
pub fn gone(p: &Palette, kind: Kind) -> Rgba {
    match kind {
        Kind::Clarity => p.gone_advice,
        Kind::Spelling | Kind::Correctness => p.gone,
    }
}

/// What a word's underline is drawn with.
pub const UNDERLINE_HEIGHT: f64 = 2.0;
/// The line's top edge is this far above the word's bottom edge, so it
/// straddles it.
pub const UNDERLINE_OFFSET: f64 = 1.0;
/// The tint and the glow reach this far past each side of the word.
pub const UNDERLINE_BLEED: f64 = 1.0;
/// Behind a word whose card is open: its underline's colour at this alpha.
pub const HOVER_FILL: f64 = 0.16;

/// The tint behind a word: the glow for one just accepted, the underline's
/// colour faintly while its card is open.
pub fn mark_fill(mark: &Mark) -> Option<Rgba> {
    let color = kind(mark.kind);
    if mark.flash {
        Some(ACCEPTED)
    } else if mark.highlighted {
        Some(Rgba {
            a: HOVER_FILL,
            ..color
        })
    } else {
        None
    }
}

/// The underline's colour; a word just accepted has none.
pub fn mark_line(mark: &Mark) -> Option<Rgba> {
    (!mark.flash).then(|| kind(mark.kind))
}

/// The card.
pub const CARD_WIDTH: f64 = 340.0;
pub const CARD_PAD: f64 = 16.0;
pub const UNDER_TITLE: f64 = 10.0;
pub const ABOVE_BUTTONS: f64 = 16.0;
pub const BETWEEN_BUTTONS: f64 = 8.0;
/// The title's line, and how far the title is indented for the dot.
pub const TITLE_LINE: f64 = 18.0;
pub const TITLE_INDENT: f64 = 15.0;
pub const DOT: f64 = 7.0;
/// A hairline: half a point is one pixel on a Retina screen.
pub const EDGE_WIDTH: f64 = 0.5;

/// The dictionary window.
pub const DICTIONARY_WIDTH: f64 = 320.0;
pub const DICTIONARY_HEIGHT: f64 = 460.0;
pub const DICTIONARY_PAD: f64 = 16.0;
pub const DICTIONARY_ROW: f64 = 36.0;

/// Every button.
pub const BUTTON_HEIGHT: f64 = 32.0;
pub const BUTTON_SIDE: f64 = 12.0;
pub const BUTTON_OUTLINE: f64 = 1.0;

/// The card's two shadows: a tight one at its edge and a soft one below. The
/// soft one dropped 18 with a 22 blur, and the card floated like a bubble.
/// `share` is the part of the palette's `shadow` strength each takes; `inset`
/// shrinks the shape that casts it.
pub struct Shadow {
    pub inset: f64,
    pub drop: f64,
    pub blur: f64,
    pub share: f64,
}

pub const SHADOW_TIGHT: Shadow = Shadow {
    inset: 0.0,
    drop: 1.0,
    blur: 2.0,
    share: 0.4,
};
pub const SHADOW_SOFT: Shadow = Shadow {
    inset: 12.0,
    drop: 14.0,
    blur: 18.0,
    share: 1.0,
};

/// How far the shadow reaches past the card: left, right, top, bottom.
pub const SHADE: (f64, f64, f64, f64) = (40.0, 40.0, 24.0, 64.0);

/// The card's one motion: it fades in over `CARD_FADE`, moving its last
/// `CARD_RISE` points towards its word. A button's fill gives way to the
/// pointer over `HOVER_FADE`: long enough to read as a response, short
/// enough that sweeping across the buttons does not trail. On Linux the
/// card just appears: the user found the motion fussy there.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub const CARD_FADE: Duration = Duration::from_millis(140);
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub const CARD_RISE: f64 = 3.0;
pub const HOVER_FADE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Medium,
    Semibold,
}

#[cfg(target_os = "linux")]
impl Weight {
    pub fn css(self) -> u16 {
        match self {
            Weight::Regular => 400,
            Weight::Medium => 500,
            Weight::Semibold => 600,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Face {
    pub size: f64,
    pub weight: Weight,
}

pub const CARD_TITLE: Face = Face {
    size: 13.0,
    weight: Weight::Medium,
};
/// The sentence, and the line it sits in.
pub const SENTENCE: Face = Face {
    size: 15.0,
    weight: Weight::Regular,
};
pub const SENTENCE_LINE: f64 = 24.0;
pub const NEW_WORDS: Face = Face {
    size: 15.0,
    weight: Weight::Semibold,
};
pub const BUTTON: Face = Face {
    size: 13.5,
    weight: Weight::Medium,
};
pub const BUTTON_MAIN: Face = Face {
    size: 13.5,
    weight: Weight::Semibold,
};
pub const DICTIONARY_TITLE: Face = Face {
    size: 15.0,
    weight: Weight::Semibold,
};
pub const DICTIONARY_HINT: Face = Face {
    size: 13.0,
    weight: Weight::Regular,
};
/// A word in the list, and what is typed in the field.
pub const DICTIONARY_WORD: Face = Face {
    size: 14.0,
    weight: Weight::Regular,
};
pub const DICTIONARY_COUNT: Face = Face {
    size: 12.5,
    weight: Weight::Regular,
};

/// Atkinson Hyperlegible Next, bundled (SIL Open Font License, see
/// assets/fonts/OFL.txt). Its I, l and 1 never look alike, which matters
/// when the fix is a capital I, and a bundled font looks the same on every
/// machine.
#[cfg(target_os = "linux")]
pub const FONT_FAMILY: &str = "Atkinson Hyperlegible Next";
pub const FONTS: [(Weight, &[u8]); 3] = [
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

/// A no-break space either side of a new word, so its fill has room to
/// breathe without touching the letters beside it.
pub const NEW_WORD_PAD: char = '\u{202F}';

/// The fill behind a new word, from the sentence font's natural height.
pub fn new_word_fill_height(natural: f64) -> f64 {
    (natural + 5.0).min(SENTENCE_LINE - 1.0).round()
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

    fn weight_name(weight: Weight) -> &'static str {
        match weight {
            Weight::Regular => "regular",
            Weight::Medium => "medium",
            Weight::Semibold => "semibold",
        }
    }

    #[test]
    fn measures_match_the_tokens() {
        let t = load();
        for (path, ours) in [
            ("card.width", CARD_WIDTH),
            ("card.padding", CARD_PAD),
            ("card.under_title", UNDER_TITLE),
            ("card.above_buttons", ABOVE_BUTTONS),
            ("card.between_buttons", BETWEEN_BUTTONS),
            ("card.title_line", TITLE_LINE),
            ("card.title_indent", TITLE_INDENT),
            ("card.dot", DOT),
            ("card.context_words", crate::surface::card::CONTEXT as f64),
            ("button.height", BUTTON_HEIGHT),
            ("button.side", BUTTON_SIDE),
            ("button.outline", BUTTON_OUTLINE),
            ("underline.height", UNDERLINE_HEIGHT),
            ("underline.offset", UNDERLINE_OFFSET),
            ("underline.bleed", UNDERLINE_BLEED),
            ("underline.hover_fill", HOVER_FILL),
            ("shadow.edge_width", EDGE_WIDTH),
            ("shadow.tight.inset", SHADOW_TIGHT.inset),
            ("shadow.tight.drop", SHADOW_TIGHT.drop),
            ("shadow.tight.blur", SHADOW_TIGHT.blur),
            ("shadow.tight.share", SHADOW_TIGHT.share),
            ("shadow.soft.inset", SHADOW_SOFT.inset),
            ("shadow.soft.drop", SHADOW_SOFT.drop),
            ("shadow.soft.blur", SHADOW_SOFT.blur),
            ("shadow.soft.share", SHADOW_SOFT.share),
            ("shadow.reach.left", SHADE.0),
            ("shadow.reach.right", SHADE.1),
            ("shadow.reach.top", SHADE.2),
            ("shadow.reach.bottom", SHADE.3),
            ("motion.card_fade_ms", CARD_FADE.as_millis() as f64),
            ("motion.card_rise", CARD_RISE),
            ("motion.hover_ms", HOVER_FADE.as_millis() as f64),
            ("type.sentence.line", SENTENCE_LINE),
        ] {
            let theirs = num(&t, path);
            assert!(
                (ours - theirs).abs() < 1e-6,
                "{path}: {ours} here, {theirs} in tokens.json"
            );
        }
    }

    #[test]
    fn type_matches_the_tokens() {
        let t = load();
        for (role, face) in [
            ("card_title", CARD_TITLE),
            ("sentence", SENTENCE),
            ("new_words", NEW_WORDS),
            ("button", BUTTON),
            ("button_main", BUTTON_MAIN),
        ] {
            assert_eq!(num(&t, &format!("type.{role}.size")), face.size, "{role}");
            assert_eq!(
                at(&t, &format!("type.{role}.weight")),
                weight_name(face.weight),
                "{role}"
            );
        }
    }

    #[test]
    fn the_bundled_fonts_are_the_ones_the_tokens_name() {
        let t = load();
        let files = at(&t, "font.files").as_str().expect("a folder");
        assert_eq!(FONTS.len(), 3);
        for (weight, _) in FONTS {
            let name = match weight {
                Weight::Regular => "Regular",
                Weight::Medium => "Medium",
                Weight::Semibold => "SemiBold",
            };
            let file = format!("{files}/AtkinsonHyperlegibleNext-{name}.ttf");
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(&file)
                    .exists(),
                "{file}"
            );
        }
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

#[cfg(test)]
mod mark_tests {
    use super::*;

    fn mark(highlighted: bool, flash: bool) -> Mark {
        Mark {
            rect: crate::surface::Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            kind: Kind::Spelling,
            highlighted,
            flash,
        }
    }

    #[test]
    fn a_word_is_tinted_while_its_card_is_open() {
        assert_eq!(mark_fill(&mark(false, false)), None);
        assert_eq!(
            mark_fill(&mark(true, false)),
            Some(Rgba {
                a: HOVER_FILL,
                ..MISTAKE
            })
        );
    }

    #[test]
    fn an_accepted_word_glows_and_has_no_line() {
        assert_eq!(mark_fill(&mark(false, true)), Some(ACCEPTED));
        assert_eq!(mark_line(&mark(false, true)), None);
        assert_eq!(mark_line(&mark(false, false)), Some(MISTAKE));
    }

    #[test]
    fn clearer_wording_takes_the_advice_colour_out() {
        assert_eq!(gone(&DARK, Kind::Clarity), DARK.gone_advice);
        assert_eq!(gone(&LIGHT, Kind::Spelling), LIGHT.gone);
        assert_eq!(gone(&LIGHT, Kind::Correctness), LIGHT.gone);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;

    #[test]
    fn the_family_is_the_tokens() {
        let t = tokens::load();
        assert_eq!(t["font"]["family"], FONT_FAMILY);
    }

    #[test]
    fn css_rounds_channels_to_bytes() {
        assert_eq!(MISTAKE.css(), "rgba(229, 72, 77, 1)");
        assert_eq!(MISTAKE.hex(), "#e5484d");
        assert_eq!(Rgba { a: 0.2, ..MISTAKE }.css(), "rgba(229, 72, 77, 0.2)");
    }
}
