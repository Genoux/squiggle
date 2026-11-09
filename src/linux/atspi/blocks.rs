//! Rich editors whose text is only their paragraphs.
//!
//! Chromium exposes each block of a contenteditable as one embedded object
//! (U+FFFC) in the field's text, with the words in the block's own text:
//! Slack's message box read "\u{fffc}", its paragraph "HIlarius! Maiking
//! SURE hes legit \u{fffc}" (the last an emoji image). Such a field is read
//! as its blocks' texts, one line each.

const OBJECT: char = '\u{fffc}';

/// Whether `raw` holds nothing but embedded objects, between spaces and
/// line breaks.
pub(super) fn only_objects(raw: &str) -> bool {
    raw.contains(OBJECT) && raw.chars().all(|c| c == OBJECT || c.is_whitespace())
}

/// Which line, of lines this long joined by line breaks, holds `len`
/// characters from `start`, and where they start in it. `None` for a range
/// that reaches past its line: a fix never crosses a line break.
pub(super) fn locate(lines: &[usize], start: usize, len: usize) -> Option<(usize, usize)> {
    let end = start.checked_add(len)?;
    lines
        .iter()
        .scan(0, |first, &line| {
            let at = *first;
            *first += line + 1;
            Some((at, line))
        })
        .enumerate()
        .find_map(|(index, (first, line))| {
            (start >= first && end <= first + line).then(|| (index, start - first))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_of_paragraphs_is_only_objects() {
        assert!(only_objects("\u{fffc}"));
        assert!(only_objects("\u{fffc}\n\u{fffc}"));
    }

    #[test]
    fn a_field_with_words_or_nothing_is_read_as_itself() {
        assert!(!only_objects(""));
        assert!(!only_objects(" \n"));
        assert!(!only_objects("thi is a lest\u{fffc}\u{fffc}"));
    }

    #[test]
    fn a_range_is_found_in_its_line() {
        // "HIlarius! Maiking" in "HIlarius! Maiking SURE hes legit \u{fffc}"
        assert_eq!(locate(&[34], 10, 7), Some((0, 10)));
        // "wrld" in "Hello\nwrld"
        assert_eq!(locate(&[5, 4], 6, 4), Some((1, 0)));
    }

    #[test]
    fn an_insertion_at_a_line_end_stays_on_that_line() {
        assert_eq!(locate(&[5, 4], 5, 0), Some((0, 5)));
    }

    #[test]
    fn a_range_across_a_line_break_or_past_the_end_is_none() {
        assert_eq!(locate(&[5, 4], 3, 4), None);
        assert_eq!(locate(&[5, 4], 8, 3), None);
        assert_eq!(locate(&[5, 4], usize::MAX, 1), None);
    }
}
