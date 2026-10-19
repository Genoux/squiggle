//! Screen geometry the engine works in: logical points, origin at the
//! top-left, y growing down - the space macOS Accessibility and Hyprland
//! both report. Each platform flips at its own edge.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Rect {
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.w
            && point.y >= self.y
            && point.y <= self.y + self.h
    }

    pub fn grow(self, by: f64) -> Rect {
        Rect {
            x: self.x - by,
            y: self.y - by,
            w: self.w + by * 2.0,
            h: self.h + by * 2.0,
        }
    }
}

/// The card, its word, and the band between them, so the pointer can travel
/// from word to card at any speed and by any route without the card closing
/// and without a word lying under the card taking over.
pub fn in_corridor(card: Rect, word: Rect, pointer: Point) -> bool {
    if card.contains(pointer) || word.grow(3.0).contains(pointer) {
        return true;
    }
    let left = card.x.min(word.x) - 4.0;
    let right = (card.x + card.w).max(word.x + word.w) + 4.0;
    let word_bottom = word.y + word.h;
    let (top, bottom) = if card.y >= word_bottom {
        (word_bottom, card.y)
    } else {
        (card.y + card.h, word.y)
    };
    pointer.x >= left && pointer.x <= right && pointer.y >= top - 2.0 && pointer.y <= bottom + 2.0
}

/// Where the card's top-left goes: close under the word, the less there is
/// to cross the less there is to fall into. Kept on the word's screen: a word
/// near the right edge of a Slack window put the sentence card half off it.
/// Shifted sideways rather than flipped, so it stays close to the word;
/// above the word only when there is no room below.
pub fn place_card(word: Rect, size: (f64, f64), area: Rect) -> Point {
    const GAP: f64 = 6.0;
    /// The card's text starts near the word: its padding.
    const SHIFT: f64 = 16.0;
    const MARGIN: f64 = 8.0;
    let (width, height) = size;
    let mut y = word.y + word.h + GAP;
    if y + height > area.y + area.h - MARGIN {
        y = (word.y - GAP - height).max(area.y + MARGIN);
    }
    let right = area.x + area.w - width - MARGIN;
    let x = (word.x - SHIFT).min(right).max(area.x + MARGIN);
    Point { x, y }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORD: Rect = Rect {
        x: 100.0,
        y: 100.0,
        w: 50.0,
        h: 20.0,
    };
    const BELOW: Rect = Rect {
        x: 90.0,
        y: 124.0,
        w: 200.0,
        h: 150.0,
    };
    const LOW_WORD: Rect = Rect {
        x: 100.0,
        y: 300.0,
        w: 50.0,
        h: 20.0,
    };
    const ABOVE: Rect = Rect {
        x: 90.0,
        y: 126.0,
        w: 200.0,
        h: 170.0,
    };
    const AREA: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1000.0,
        h: 800.0,
    };

    fn at(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn corridor_holds_the_band_below_the_word() {
        assert!(in_corridor(BELOW, WORD, at(120.0, 122.0)));
    }

    #[test]
    fn corridor_holds_the_band_above_the_word() {
        assert!(in_corridor(ABOVE, LOW_WORD, at(120.0, 298.0)));
    }

    #[test]
    fn corridor_holds_the_word_and_the_card() {
        assert!(in_corridor(BELOW, WORD, at(110.0, 110.0)));
        assert!(in_corridor(BELOW, WORD, at(150.0, 200.0)));
        assert!(in_corridor(BELOW, WORD, at(98.0, 110.0)));
    }

    #[test]
    fn corridor_excludes_beside_the_band() {
        assert!(!in_corridor(BELOW, WORD, at(300.0, 122.0)));
        assert!(!in_corridor(ABOVE, LOW_WORD, at(300.0, 298.0)));
    }

    #[test]
    fn corridor_excludes_past_the_card() {
        assert!(!in_corridor(BELOW, WORD, at(120.0, 400.0)));
        assert!(!in_corridor(BELOW, WORD, at(120.0, 50.0)));
    }

    #[test]
    fn corridor_needs_both_card_and_word_to_be_real_rects() {
        let away = Rect { x: 500.0, ..WORD };
        assert!(!in_corridor(BELOW, away, at(700.0, 300.0)));
    }

    #[test]
    fn card_goes_below_the_word_when_there_is_room() {
        assert_eq!(
            place_card(WORD, (200.0, 150.0), AREA),
            Point { x: 84.0, y: 126.0 }
        );
    }

    #[test]
    fn card_flips_above_at_the_bottom_edge() {
        let word = Rect { y: 700.0, ..WORD };
        assert_eq!(
            place_card(word, (200.0, 150.0), AREA),
            Point { x: 84.0, y: 544.0 }
        );
    }

    #[test]
    fn card_above_stays_inside_the_top_edge() {
        let word = Rect { y: 60.0, ..WORD };
        let short = Rect { h: 100.0, ..AREA };
        assert_eq!(
            place_card(word, (200.0, 150.0), short),
            Point { x: 84.0, y: 8.0 }
        );
    }

    #[test]
    fn card_is_shifted_in_from_the_right_edge() {
        let word = Rect { x: 950.0, ..WORD };
        assert_eq!(
            place_card(word, (200.0, 150.0), AREA),
            Point { x: 792.0, y: 126.0 }
        );
    }

    #[test]
    fn card_is_shifted_in_from_the_left_edge() {
        let word = Rect { x: 0.0, ..WORD };
        assert_eq!(
            place_card(word, (200.0, 150.0), AREA),
            Point { x: 8.0, y: 126.0 }
        );
    }

    #[test]
    fn rect_grows_on_every_side() {
        assert_eq!(
            WORD.grow(2.0),
            Rect {
                x: 98.0,
                y: 98.0,
                w: 54.0,
                h: 24.0
            }
        );
    }
}
