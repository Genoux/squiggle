//! One suggested edit, in the shape every surface draws and applies.
//!
//! Offsets are in `char`s because that is what Harper reports and what the
//! model's edits are located in. The platforms do not speak chars: macOS
//! Accessibility counts UTF-16 code units, AT-SPI counts chars, and an emoji
//! earlier in the field moves every underline after it by one if the two are
//! mixed up. [`utf16_range`] is the only conversion, so there is one place to be
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
        utf16_range(text, self.start, self.end)
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

/// The char range `start..end` of `text` as `(location, length)` in UTF-16
/// code units.
pub fn utf16_range(text: &str, start: usize, end: usize) -> (usize, usize) {
    let mut location = 0;
    let mut length = 0;
    for (index, c) in text.chars().enumerate() {
        if index < start {
            location += c.len_utf16();
        } else if index < end {
            length += c.len_utf16();
        } else {
            break;
        }
    }
    (location, length)
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

/// Carries fixes across an edit so underlines stay put while the user types.
///
/// The edit is found as the span between the longest common prefix and
/// suffix of the two texts - one contiguous change, which is what a keystroke,
/// a paste, or an accepted fix is. Fixes before it keep their offsets, fixes
/// after it shift by the change in length, and a fix the edit touched is
/// dropped: its words are not the words it was about any more. Clearing every
/// underline on every keystroke instead is what made them flicker.
pub fn rebase(old: &str, new: &str, fixes: &[Fix]) -> Vec<Fix> {
    let a: Vec<char> = old.chars().collect();
    let b: Vec<char> = new.chars().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let max_suffix = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(max_suffix)
        .take_while(|(x, y)| x == y)
        .count();
    let old_end = a.len() - suffix;
    let delta = b.len() as isize - a.len() as isize;
    let removed = &a[prefix..old_end];
    let inserted = &b[prefix..b.len() - suffix];
    // A change touching a word's edge changes the word when it adds or
    // removes letters there ("teh" → "tehh"), and does not when it is a
    // space or punctuation ("teh" → "teh,").
    let wordy = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '\'');
    let joins_left = wordy(inserted.first()) || wordy(removed.first());
    let joins_right = wordy(inserted.last()) || wordy(removed.last());
    fixes
        .iter()
        .filter_map(|fix| {
            let before = fix.end < prefix || (fix.end == prefix && !joins_left);
            let after = fix.start > old_end || (fix.start == old_end && !joins_right);
            if before && fix.end <= prefix {
                Some(fix.clone())
            } else if after && fix.start >= old_end {
                let mut moved = fix.clone();
                moved.start = (fix.start as isize + delta) as usize;
                moved.end = (fix.end as isize + delta) as usize;
                Some(moved)
            } else {
                None
            }
        })
        .collect()
}

/// The sentence around char `at`, as `(start, end)` char offsets. A sentence
/// ends after `.`, `!`, `?` or `…` followed by whitespace, or at a line break.
/// Good enough to group fixes for "Correct the sentence"; an abbreviation
/// ("e.g. this") splits a sentence early, which only means a smaller group.
pub fn sentence(text: &str, at: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let at = at.min(chars.len());
    let ends = |i: usize| {
        matches!(chars[i], '.' | '!' | '?' | '…')
            && chars.get(i + 1).is_none_or(|c| c.is_whitespace())
    };
    let mut start = at;
    while start > 0 && chars[start - 1] != '\n' && !ends(start - 1) {
        start -= 1;
    }
    while start < at && chars[start].is_whitespace() {
        start += 1;
    }
    let mut end = at;
    while end < chars.len() && chars[end] != '\n' {
        end += 1;
        if ends(end - 1) {
            break;
        }
    }
    (start, end)
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
    fn utf16_range_counts_chars_up_to_each_end() {
        assert_eq!(utf16_range("😀 teh cat", 2, 5), (3, 3));
        assert_eq!(utf16_range("a😀b", 1, 2), (1, 2));
        assert_eq!(utf16_range("a😀b", 3, 3), (4, 0));
        assert_eq!(utf16_range("ab", 1, 9), (1, 1));
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
    fn typing_after_a_mistake_leaves_it_underlined() {
        let old = "teh cat";
        let fixes = [fix(0, 3, "the", Source::Local)];
        assert_eq!(rebase(old, "teh cats", &fixes), fixes.to_vec());
        assert_eq!(rebase("teh", "teh ", &fixes), fixes.to_vec());
    }

    #[test]
    fn typing_before_a_mistake_moves_it() {
        let old = "a teh cat";
        let fixes = [fix(2, 5, "the", Source::Local)];
        let new = "and a teh cat";
        let moved = rebase(old, new, &fixes);
        assert_eq!(moved[0].original(new), "teh");
    }

    #[test]
    fn editing_the_mistake_drops_it() {
        let fixes = [
            fix(0, 3, "the", Source::Local),
            fix(8, 11, "the", Source::Local),
        ];
        let old = "teh cat teh dog";
        let new = "tehh cat teh dog";
        let kept = rebase(old, new, &fixes);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].original(new), "teh");
    }

    #[test]
    fn accepting_one_fix_keeps_the_others_in_place() {
        let old = "teh cat and teh dog";
        let fixes = [
            fix(0, 3, "the", Source::Local),
            fix(12, 15, "the", Source::Local),
        ];
        let new = apply(old, &fixes[..1]);
        let kept = rebase(old, &new, &fixes);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].original(&new), "teh");
    }

    #[test]
    fn sentence_bounds() {
        let text = "Cool and nice. mais je pourai pas faire l'onneur. Fin";
        let at = text.find("pourai").unwrap();
        let (start, end) = sentence(text, at);
        let found: String = text.chars().skip(start).take(end - start).collect();
        assert_eq!(found, "mais je pourai pas faire l'onneur.");
        let (start, end) = sentence(text, text.len());
        let last: String = text.chars().skip(start).take(end - start).collect();
        assert_eq!(last, "Fin");
    }

    #[test]
    fn insertion_at_an_edge_is_not_an_overlap() {
        let replace = fix(0, 3, "X", Source::Local);
        let insert = fix(3, 3, ",", Source::Ai);
        assert!(!replace.overlaps(&insert));
    }
}
