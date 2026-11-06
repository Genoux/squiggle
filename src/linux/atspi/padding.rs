//! Gecko's padding after astral characters, and the offsets it shifts.

/// Gecko counts UTF-16 and pads each astral character with a U+FEFF; GTK4
/// and Chromium count characters. The text Squiggle reads has the padding
/// removed, so offsets are characters everywhere, and this is where each
/// piece sat: the position in the stripped text that each removed U+FEFF
/// stood before.
pub(super) fn strip_padding(raw: &str) -> (String, Vec<usize>) {
    let mut padding = Vec::new();
    let mut text = String::with_capacity(raw.len());
    let mut kept = 0;
    for character in raw.chars() {
        if character == '\u{feff}' {
            padding.push(kept);
        } else {
            text.push(character);
            kept += 1;
        }
    }
    (text, padding)
}

/// Where the app counts the character Squiggle counts at `offset`.
pub(super) fn app_offset(padding: &[usize], offset: usize) -> usize {
    offset + padding.iter().filter(|&&at| at <= offset).count()
}

/// The app's offsets for `len` characters from `start`. A padding that
/// follows the range's last character is inside it: deleting an emoji
/// without its U+FEFF left the padding behind.
pub(super) fn app_range(padding: &[usize], start: usize, len: usize) -> Option<(i32, i32)> {
    let end = start.checked_add(len)?;
    Some((
        i32::try_from(app_offset(padding, start)).ok()?,
        i32::try_from(app_offset(padding, end)).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_without_padding_maps_to_itself() {
        let (text, padding) = strip_padding("Hello wrold");
        assert_eq!(text, "Hello wrold");
        assert_eq!(app_offset(&padding, 6), 6);
    }

    #[test]
    fn gecko_pads_an_emoji_with_a_byte_order_mark() {
        let (text, padding) = strip_padding("Hello 😀\u{feff} wrold");
        assert_eq!(text, "Hello 😀 wrold");
        assert_eq!(app_offset(&padding, 5), 5);
        assert_eq!(app_offset(&padding, 6), 6);
        assert_eq!(app_offset(&padding, 7), 8);
        assert_eq!(app_offset(&padding, 8), 9);
    }

    #[test]
    fn every_padded_emoji_shifts_what_follows() {
        let (text, padding) = strip_padding("😀\u{feff}😀\u{feff}x");
        assert_eq!(text, "😀😀x");
        assert_eq!(app_offset(&padding, 0), 0);
        assert_eq!(app_offset(&padding, 1), 2);
        assert_eq!(app_offset(&padding, 2), 4);
    }

    #[test]
    fn a_range_starting_after_the_padding_moves_whole() {
        let (_, padding) = strip_padding("😀\u{feff} ab");
        assert_eq!(app_offset(&padding, 2), 3);
        assert_eq!(app_offset(&padding, 3), 4);
    }

    #[test]
    fn a_range_without_padding_is_its_own() {
        assert_eq!(app_range(&[], 3, 2), Some((3, 5)));
        assert_eq!(app_range(&[], 3, 0), Some((3, 3)));
    }

    #[test]
    fn a_range_ending_on_a_padded_emoji_takes_the_padding() {
        let (_, padding) = strip_padding("😀\u{feff} ab");
        assert_eq!(app_range(&padding, 0, 1), Some((0, 2)));
    }

    #[test]
    fn a_range_after_a_padded_emoji_moves_whole() {
        let (_, padding) = strip_padding("😀\u{feff} wrold");
        assert_eq!(app_range(&padding, 2, 5), Some((3, 8)));
    }

    #[test]
    fn a_range_across_a_padded_emoji_grows_by_its_padding() {
        let (_, padding) = strip_padding("a 😀\u{feff} b");
        assert_eq!(app_range(&padding, 0, 5), Some((0, 6)));
    }

    #[test]
    fn a_range_past_what_an_offset_can_hold_is_none() {
        assert_eq!(app_range(&[], usize::MAX, 1), None);
    }
}
