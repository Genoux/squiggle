//! A corrected text back into fixes: a word-level diff of the original
//! against the model's correction.
//!
//! Tokens are words (letters, digits, apostrophes, inner hyphens), runs of
//! whitespace, and single punctuation marks, so "think" → "think?" is one
//! small change rather than a new word. Changes separated only by whitespace
//! merge into one, which is how "jpense" → "je pense que" stays one fix.

use crate::fix::{Fix, Kind, Source};

/// Past this many tokens either side, the paragraph is not diffed: the table
/// is quadratic and no one writes a paragraph that long by hand.
const MOST_TOKENS: usize = 2000;
/// One fix spanning more than this is a rewrite, not a correction.
const LONGEST_FIX: usize = 80;
/// A correction touching more than this share of a paragraph's words (of at
/// least `SHORTEST_GUARDED` words) rewrote it; nothing of it is shown.
const MOST_CHANGED: f64 = 0.6;
const SHORTEST_GUARDED: usize = 5;

#[derive(Debug, Clone, PartialEq)]
struct Token {
    text: String,
    start: usize,
    end: usize,
}

impl Token {
    fn space(&self) -> bool {
        self.text.chars().all(char::is_whitespace)
    }
}

fn wordish(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '\'' | '’')
}

fn tokens(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        let c = chars[i];
        if wordish(c) {
            while i < chars.len()
                && (wordish(chars[i])
                    || (chars[i] == '-'
                        && i + 1 < chars.len()
                        && wordish(chars[i + 1])
                        && i > start))
            {
                i += 1;
            }
        } else if c.is_whitespace() {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
        } else {
            i += 1;
        }
        out.push(Token {
            text: chars[start..i].iter().collect(),
            start,
            end: i,
        });
    }
    out
}

/// Hunks of `(a_lo, a_hi, b_lo, b_hi)` token ranges that differ.
fn hunks(a: &[Token], b: &[Token]) -> Vec<(usize, usize, usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i].text == b[j].text {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut open: Option<(usize, usize)> = None;
    while i < n || j < m {
        if i < n && j < m && a[i].text == b[j].text {
            if let Some((ai, bj)) = open.take() {
                out.push((ai, i, bj, j));
            }
            i += 1;
            j += 1;
        } else {
            open.get_or_insert((i, j));
            if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
                j += 1;
            } else {
                i += 1;
            }
        }
    }
    if let Some((ai, bj)) = open {
        out.push((ai, n, bj, m));
    }

    // Changes with only whitespace between them are one change.
    let mut merged: Vec<(usize, usize, usize, usize)> = Vec::new();
    for hunk in out {
        if let Some(last) = merged.last_mut()
            && a[last.1..hunk.0].iter().all(Token::space)
            && b[last.3..hunk.2].iter().all(Token::space)
        {
            last.1 = hunk.1;
            last.3 = hunk.3;
            continue;
        }
        merged.push(hunk);
    }
    merged
}

fn joined(tokens: &[Token]) -> String {
    tokens.iter().map(|t| t.text.as_str()).collect()
}

/// What kind of change, and the card's title for it - generic, as
/// Grammarly's are ("Correct your spelling"): the change itself is the
/// explanation, and a title naming the rule read as a lecture.
fn classify(original: &str, replacement: &str) -> (Kind, String) {
    let letters = |s: &str| -> String { s.chars().filter(|c| c.is_alphanumeric()).collect() };
    let same_letters = letters(original).to_lowercase() == letters(replacement).to_lowercase();
    let case = letters(original) != letters(replacement);
    let (title, kind) = if same_letters && case {
        ("Correct the capitalization", Kind::Correctness)
    } else if same_letters {
        ("Correct your punctuation", Kind::Correctness)
    } else if !original.is_empty()
        && original.chars().all(wordish)
        && replacement.chars().all(wordish)
        && !replacement.is_empty()
    {
        ("Correct your spelling", Kind::Spelling)
    } else {
        ("Correct your grammar", Kind::Correctness)
    };
    (kind, title.into())
}

/// Fixes that turn `original` into `corrected`, or none if the correction
/// rewrote rather than corrected.
///
/// Diffed line by line: a fix never crosses or removes a line break. Diffing
/// the whole text let "think\n" → "think?" come back as a fix replacing the
/// line break with "?", which accepted would have joined two lines. If the
/// model changed the number of lines, nothing is trusted.
pub fn fixes(original: &str, corrected: &str) -> Vec<Fix> {
    let mut corrected = corrected.to_string();
    // The model often drops a final line break; that is not a correction.
    while original.ends_with('\n') && !corrected.ends_with('\n') {
        corrected.push('\n');
    }
    let old: Vec<&str> = original.split('\n').collect();
    let new: Vec<&str> = corrected.split('\n').collect();
    if old.len() != new.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut offset = 0;
    for (a, b) in old.iter().zip(&new) {
        out.extend(line_fixes(a, b).into_iter().map(|mut fix| {
            fix.start += offset;
            fix.end += offset;
            fix
        }));
        offset += a.chars().count() + 1;
    }
    out
}

fn line_fixes(original: &str, corrected: &str) -> Vec<Fix> {
    let a = tokens(original);
    let b = tokens(corrected);
    if a.len() > MOST_TOKENS || b.len() > MOST_TOKENS {
        return Vec::new();
    }
    let found = hunks(&a, &b);

    let words = a.iter().filter(|t| !t.space()).count();
    let changed: usize = found
        .iter()
        .map(|h| a[h.0..h.1].iter().filter(|t| !t.space()).count())
        .sum();
    if words >= SHORTEST_GUARDED && changed as f64 > words as f64 * MOST_CHANGED {
        return Vec::new();
    }

    let mut out = Vec::new();
    for (a_lo, a_hi, b_lo, b_hi) in found {
        let (mut a_lo, mut b_lo, mut a_hi, mut b_hi) = (a_lo, b_lo, a_hi, b_hi);
        // Line breaks never reach here (lines are diffed apart), so a
        // whitespace change is spacing inside a line: a doubled space is a
        // mistake; trailing space at the end of a line is the writer
        // mid-sentence, not one.
        let spacing =
            a[a_lo..a_hi].iter().all(Token::space) && b[b_lo..b_hi].iter().all(Token::space);
        if spacing && a_hi == a.len() {
            continue;
        }
        // Nothing to underline for a pure insertion, so it takes the word
        // before it along ("think" → "think?"), or after it at the start.
        // Spacing stays on the spaces: attached to the word before, it
        // overlapped that word's own spelling fix and replaced it.
        if a_lo == a_hi {
            if let Some(prev) = (0..a_lo).rev().find(|&k| !a[k].space()) {
                b_lo -= a_lo - prev;
                a_lo = prev;
            } else if let Some(next) = (a_hi..a.len()).find(|&k| !a[k].space()) {
                b_hi += next + 1 - a_hi;
                a_hi = next + 1;
            } else {
                continue;
            }
        }
        // Leading or trailing whitespace inside a hunk is not part of it.
        while a_lo < a_hi && b_lo < b_hi && a[a_lo].space() && a[a_lo].text == b[b_lo].text {
            a_lo += 1;
            b_lo += 1;
        }
        while a_hi > a_lo
            && b_hi > b_lo
            && a[a_hi - 1].space()
            && a[a_hi - 1].text == b[b_hi - 1].text
        {
            a_hi -= 1;
            b_hi -= 1;
        }
        if a_lo >= a_hi {
            continue;
        }
        let was = joined(&a[a_lo..a_hi]);
        let now = joined(&b[b_lo..b_hi]);
        if was == now || was.chars().count() > LONGEST_FIX {
            continue;
        }
        let (kind, message) = classify(was.trim(), now.trim());
        out.push(Fix {
            start: a[a_lo].start,
            end: a[a_hi - 1].end,
            replacement: now,
            kind,
            message,
            source: Source::Ai,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fix::apply;

    fn round_trip(original: &str, corrected: &str) -> Vec<Fix> {
        let found = fixes(original, corrected);
        assert_eq!(apply(original, &found), corrected, "{found:#?}");
        found
    }

    #[test]
    fn capitals_and_end_punctuation() {
        let found = round_trip(
            "i went to paris. what do you think",
            "I went to Paris. What do you think?",
        );
        let originals: Vec<String> = found
            .iter()
            .map(|f| f.original("i went to paris. what do you think"))
            .collect();
        assert_eq!(originals, ["i", "paris", "what", "think"]);
        assert_eq!(found[3].replacement, "think?");
        assert_eq!(found[0].message, "Correct the capitalization");
    }

    #[test]
    fn a_split_word_with_an_added_one_is_one_fix() {
        let text = "Mais jpense je vois la lune au petite solail.";
        let found = round_trip(text, "Mais je pense que je vois la lune au petit soleil.");
        let originals: Vec<String> = found.iter().map(|f| f.original(text)).collect();
        assert_eq!(originals, ["jpense", "petite solail"]);
    }

    #[test]
    fn unchanged_text_has_no_fixes() {
        assert!(fixes("All good here.", "All good here.").is_empty());
    }

    #[test]
    fn a_dropped_trailing_newline_is_not_a_fix() {
        assert!(fixes("All good here.\n", "All good here.").is_empty());
    }

    #[test]
    fn a_rewrite_is_refused() {
        let text = "I think we should be able to solve this issue for you today.";
        assert!(fixes(text, "We can fix it for you right now, no problem at all.").is_empty());
    }

    #[test]
    fn a_line_break_is_never_part_of_a_fix() {
        let text = "what do you think\nsee you\n";
        let found = fixes(text, "What do you think?\nSee you.");
        assert_eq!(apply(text, &found), "What do you think?\nSee you.\n");
        assert!(
            found.iter().all(|f| !f.original(text).contains('\n')),
            "{found:#?}"
        );
    }

    #[test]
    fn a_doubled_space_is_a_fix_on_the_spaces() {
        let text = "on the wike  side";
        let found = round_trip(text, "on the wike side");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].original(text), "  ");
    }

    #[test]
    fn trailing_space_while_typing_is_not_a_fix() {
        assert!(fixes("still typing ", "still typing").is_empty());
    }

    #[test]
    fn a_misspelled_word_is_spelling() {
        let found = round_trip("I recieve mail.", "I receive mail.");
        assert_eq!(found[0].kind, Kind::Spelling);
    }

    #[test]
    fn apostrophes_stay_inside_words() {
        let text = "C'est cool mais jpense pas.";
        let found = round_trip(text, "C'est cool mais je pense pas.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].original(text), "jpense");
    }
}
