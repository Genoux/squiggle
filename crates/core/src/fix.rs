//! One suggested edit, in the shape every surface draws and applies.
//!
//! Offsets are in `char`s because that is what Harper reports and what the
//! model's edits are located in. The platforms do not speak chars: macOS
//! Accessibility counts UTF-16 code units, AT-SPI counts chars, and an emoji
//! earlier in the field moves every underline after it by one if the two are
//! mixed up. [`Fix::utf16`] is the only conversion, so there is one place to be
//! wrong instead of one per backend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// The word is not in the dictionary. Drawn red, and the only kind the
    /// card offers to learn.
    Spelling,
    /// The word is a word and still wrong here. Drawn red.
    Correctness,
    /// The sentence is grammatical and could still read better. Drawn blue.
    Clarity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// Harper, on this machine, on every pause.
    Local,
    /// The model, a second or so after the local pass.
    Ai,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fix {
    /// First char of the text being replaced.
    pub start: usize,
    /// One past the last char. `start == end` is an insertion.
    pub end: usize,
    pub replacement: String,
    pub kind: Kind,
    /// One short line for the card - why, not what.
    pub message: String,
    pub source: Source,
}

impl Fix {
    /// The text this fix replaces, read out of the text it was found in.
    pub fn original(&self, text: &str) -> String {
        text.chars()
            .skip(self.start)
            .take(self.end - self.start)
            .collect()
    }

    /// `(location, length)` in UTF-16 code units, which is what macOS
    /// `AXBoundsForRange` and `AXSelectedTextRange` take.
    pub fn utf16(&self, text: &str) -> (usize, usize) {
        let mut location = 0;
        let mut length = 0;
        for (index, c) in text.chars().enumerate() {
            if index < self.start {
                location += c.len_utf16();
            } else if index < self.end {
                length += c.len_utf16();
            } else {
                break;
            }
        }
        (location, length)
    }

    fn overlaps(&self, other: &Fix) -> bool {
        // Two insertions at the same point collide; an insertion touching the
        // edge of a replacement does not.
        if self.start == self.end && other.start == other.end {
            return self.start == other.start;
        }
        self.start < other.end && other.start < self.end
    }
}

/// The local pass and the model's pass as one list, ordered by position.
///
/// Where they disagree about the same words, the model wins. Harper finds a
/// misspelling exactly but chooses its replacement from the dictionary alone,
/// so "I red the book" is underlined correctly and offered "red" → "rad"; the
/// model read the sentence. Where only one of them found something, it stays.
pub fn merge(local: Vec<Fix>, ai: Vec<Fix>) -> Vec<Fix> {
    let mut merged: Vec<Fix> = ai;
    for fix in local {
        if !merged.iter().any(|kept| kept.overlaps(&fix)) {
            merged.push(fix);
        }
    }
    merged.sort_by_key(|fix| (fix.start, fix.end));
    merged
}

/// `text` with every fix applied. Fixes that overlap one already applied are
/// skipped rather than guessed at, since either would leave the other's
/// offsets pointing at the wrong words.
pub fn apply(text: &str, fixes: &[Fix]) -> String {
    let mut chosen: Vec<&Fix> = Vec::new();
    let mut sorted: Vec<&Fix> = fixes.iter().collect();
    sorted.sort_by_key(|fix| (fix.start, fix.end));
    for fix in sorted {
        if !chosen.iter().any(|kept| kept.overlaps(fix)) {
            chosen.push(fix);
        }
    }

    let mut chars: Vec<char> = text.chars().collect();
    // Back to front, so an applied fix never moves the offsets of the next.
    for fix in chosen.into_iter().rev() {
        if fix.end > chars.len() || fix.start > fix.end {
            continue;
        }
        chars.splice(fix.start..fix.end, fix.replacement.chars());
    }
    chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(start: usize, end: usize, replacement: &str, source: Source) -> Fix {
        Fix {
            start,
            end,
            replacement: replacement.into(),
            kind: Kind::Correctness,
            message: String::new(),
            source,
        }
    }

    #[test]
    fn utf16_counts_an_emoji_as_two() {
        let text = "😀 teh cat";
        let typo = fix(2, 5, "the", Source::Local);
        assert_eq!(typo.original(text), "teh");
        assert_eq!(typo.utf16(text), (3, 3));
    }

    #[test]
    fn apply_keeps_later_offsets_valid() {
        let text = "teh cat sat on teh mat";
        let fixes = [
            fix(0, 3, "the", Source::Local),
            fix(15, 18, "the", Source::Local),
        ];
        assert_eq!(apply(text, &fixes), "the cat sat on the mat");
    }

    #[test]
    fn apply_handles_a_longer_replacement_first() {
        let text = "its a dog its";
        let fixes = [
            fix(0, 3, "it's", Source::Local),
            fix(10, 13, "it's", Source::Local),
        ];
        assert_eq!(apply(text, &fixes), "it's a dog it's");
    }

    #[test]
    fn apply_skips_an_overlapping_fix() {
        let text = "abcdef";
        let fixes = [fix(0, 3, "X", Source::Local), fix(2, 4, "Y", Source::Ai)];
        assert_eq!(apply(text, &fixes), "Xdef");
    }

    #[test]
    fn merge_prefers_the_model_on_the_same_words() {
        let local = vec![
            fix(2, 5, "rad", Source::Local),
            fix(10, 13, "the", Source::Local),
        ];
        let ai = vec![fix(2, 5, "read", Source::Ai)];
        let merged = merge(local, ai);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].replacement, "read");
        assert_eq!(merged[1].replacement, "the");
    }

    #[test]
    fn insertion_at_an_edge_is_not_an_overlap() {
        let replace = fix(0, 3, "X", Source::Local);
        let insert = fix(3, 3, ",", Source::Ai);
        assert!(!replace.overlaps(&insert));
    }
}
