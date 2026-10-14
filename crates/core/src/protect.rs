//! Text that is not prose and is never corrected: links, domains, email
//! addresses, @mentions, #tags, file paths, `code`, and identifiers like
//! `AdCollection` or `snake_case`. Grammarly leaves them alone too.
//!
//! The model is asked to leave them as written, and does not always: it
//! offered "frame.io" -> "frame.i". So every fix, the model's and Harper's,
//! is checked against these spans and one that would change them is dropped.
//! A fix that only adds around them stays: "frame.io" -> "frame.io." is the
//! sentence's missing period, attached to the word before it by `diff`.
//!
//! A bare domain may still change case. "frame.io" is a product, and
//! Grammarly offers "Frame.io" for it, as it capitalises any product name;
//! a link, an address or a path is never touched, since case can break it.

use crate::fix::Fix;

/// `fixes` without the ones that would change protected text in `text`.
pub fn keep(text: &str, fixes: Vec<Fix>) -> Vec<Fix> {
    filter(text, fixes, &spans(text))
}

/// `fixes` without the ones that would change a word of the user's
/// dictionary. Harper is given the words and never flags them; the model is
/// not, so with AI on a word added from a card was still corrected on the
/// next pause. Checked after the model answers, so a word added applies at
/// once, cached answers included, and the dictionary never leaves the Mac.
pub fn keep_known(text: &str, fixes: Vec<Fix>, words: &[String]) -> Vec<Fix> {
    if words.is_empty() {
        return fixes;
    }
    let spans: Vec<Span> = tokens(text)
        .into_iter()
        .filter_map(|(start, end, word)| {
            let lower = word.to_lowercase();
            let bare = lower
                .strip_suffix("'s")
                .or_else(|| lower.strip_suffix("\u{2019}s"))
                .unwrap_or(&lower);
            let entry = words.iter().find(|w| {
                let w = w.to_lowercase();
                w == lower || w == bare
            })?;
            Some(Span {
                start,
                end,
                // "hyprland" may still become the dictionary's "Hyprland".
                case_free: !word.starts_with(entry.as_str()),
            })
        })
        .collect();
    filter(text, fixes, &spans)
}

fn filter(text: &str, fixes: Vec<Fix>, spans: &[Span]) -> Vec<Fix> {
    if spans.is_empty() {
        return fixes;
    }
    fixes
        .into_iter()
        .filter(|fix| {
            let touched: Vec<&Span> = spans
                .iter()
                .filter(|span| {
                    fix.start < span.end && span.start < fix.end
                        // An insertion inside a span changes it too.
                        || fix.start == fix.end && span.start < fix.start && fix.start < span.end
                })
                .collect();
            if touched.is_empty() {
                return true;
            }
            let original = fix.original(text);
            let keeps = |original: &str, replacement: &str| {
                !original.is_empty()
                    && (replacement.starts_with(original) || replacement.ends_with(original))
            };
            keeps(&original, &fix.replacement)
                || touched.iter().all(|span| span.case_free)
                    && keeps(&original.to_lowercase(), &fix.replacement.to_lowercase())
        })
        .collect()
}

/// A run of `text` that is not prose, in chars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    /// A bare domain, which may be capitalised as a product name.
    pub case_free: bool,
}

/// The runs of `text` that are not prose.
pub fn spans(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = code_spans(&chars);
    for (start, end, word) in tokens(text) {
        if protected(&word) {
            out.push(Span {
                start,
                end,
                case_free: domain(&word) && !word.contains('/'),
            });
        }
    }
    out.sort_unstable();
    out
}

/// Each run of non-space in `text` without the punctuation around it, in
/// chars: "(frame.io)." is "frame.io".
fn tokens(text: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < chars.len() && !chars[end].is_whitespace() {
            end += 1;
        }
        // Punctuation around a token belongs to the sentence: "(frame.io)."
        let mut start = i;
        let mut stop = end;
        while start < stop && OPENING.contains(&chars[start]) {
            start += 1;
        }
        while stop > start && CLOSING.contains(&chars[stop - 1]) {
            stop -= 1;
        }
        if start < stop {
            out.push((start, stop, chars[start..stop].iter().collect()));
        }
        i = end;
    }
    out
}

const OPENING: &[char] = &['(', '[', '{', '"', '\'', '«', '“', '‘', '<'];
const CLOSING: &[char] = &[
    '.', ',', ';', ':', '!', '?', ')', ']', '}', '"', '\'', '»', '”', '’', '…', '>',
];

/// Text between a pair of backticks on one line.
fn code_spans(chars: &[char]) -> Vec<Span> {
    let mut out = Vec::new();
    let mut open: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        match (c, open) {
            ('`', None) => open = Some(i),
            ('`', Some(start)) => {
                out.push(Span {
                    start,
                    end: i + 1,
                    case_free: false,
                });
                open = None;
            }
            ('\n', _) => open = None,
            _ => {}
        }
    }
    out
}

fn protected(word: &str) -> bool {
    if word.chars().count() < 2 {
        return false;
    }
    word.contains("://")
        || word.starts_with("www.")
        || word.starts_with(['@', '#']) && word[1..].starts_with(|c: char| c.is_alphanumeric())
        || email(word)
        || path(word)
        || domain(word)
        || identifier(word)
}

fn email(word: &str) -> bool {
    word.split_once('@')
        .is_some_and(|(user, host)| !user.is_empty() && domain(host))
}

fn path(word: &str) -> bool {
    ["/", "~/", "./", "../"].iter().any(|p| word.starts_with(p)) && word.len() > 2
}

/// `frame.io`, `docs.rs/serde`. The last label is lowercase letters, which
/// keeps a missing space after a full stop ("draft.Pick") a typo to fix.
fn domain(word: &str) -> bool {
    let host = word.split('/').next().unwrap_or(word);
    let labels: Vec<&str> = host.split('.').collect();
    let Some(tld) = labels.last() else {
        return false;
    };
    labels.len() >= 2
        && (2..=12).contains(&tld.len())
        && tld.chars().all(|c| c.is_ascii_lowercase())
        && labels.iter().all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
}

/// `AdCollection`, `iPhone`, `snake_case`: a capital after a lowercase
/// letter, or an underscore between letters, is a name a writer chose.
/// A lone capital on the end of a lowercase word is not: "congelationÉ"
/// was a French-keyboard slip for "?", and protecting it dropped the
/// model's fix.
fn identifier(word: &str) -> bool {
    if !word.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return false;
    }
    let chars: Vec<char> = word.chars().collect();
    let slip = chars.len() > 3
        && chars.last().is_some_and(|c| c.is_uppercase())
        && !chars[..chars.len() - 1].iter().any(|c| c.is_uppercase());
    !slip
        && chars
            .windows(2)
            .any(|pair| pair[0].is_lowercase() && pair[1].is_uppercase())
        || word.trim_matches('_').contains('_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fix::{Kind, Source};

    fn found(text: &str) -> Vec<String> {
        spans(text)
            .into_iter()
            .map(|span| {
                text.chars()
                    .skip(span.start)
                    .take(span.end - span.start)
                    .collect()
            })
            .collect()
    }

    fn fix(text: &str, original: &str, replacement: &str) -> Fix {
        let byte = text.find(original).unwrap();
        let start = text[..byte].chars().count();
        Fix {
            start,
            end: start + original.chars().count(),
            replacement: replacement.into(),
            kind: Kind::Correctness,
            message: String::new(),
            source: Source::Ai,
        }
    }

    #[test]
    fn a_dictionary_word_is_never_changed() {
        let words = vec!["Hyprland".to_string(), "tmrw".to_string()];
        let text = "I run hyprland and Hyprland's bar, see you tmrw";
        let fixes = vec![
            fix(text, "run", "ran"),
            fix(text, "hyprland", "Hyprland"),
            fix(text, "Hyprland's", "Hyperland's"),
            fix(text, "tmrw", "tomorrow"),
        ];
        let kept: Vec<String> = keep_known(text, fixes, &words)
            .into_iter()
            .map(|f| f.replacement)
            .collect();
        assert_eq!(kept, ["ran", "Hyprland"]);
    }

    #[test]
    fn finds_what_is_not_prose() {
        assert_eq!(
            found(
                "AdCollection Pick an old ad from frame.io. Mail me@x.com or @sam, see #42 and `cargo run` in ~/src (https://a.b/c)."
            ),
            vec![
                "AdCollection",
                "frame.io",
                "me@x.com",
                "@sam",
                "#42",
                "`cargo run`",
                "~/src",
                "https://a.b/c",
            ]
        );
        assert_eq!(found("snake_case and iPhone"), vec!["snake_case", "iPhone"]);
        assert_eq!(found("pH, macOS and kWh"), vec!["pH", "macOS", "kWh"]);
    }

    #[test]
    fn a_capital_stuck_on_a_word_is_a_slip() {
        assert!(found("fermentation or congelationÉ").is_empty());
    }

    #[test]
    fn leaves_prose_alone() {
        assert!(found("I went to Paris. Then e.g. and/or, it's fine... OK? #").is_empty());
        assert!(found("Create a new ads draft.Pick an old ad").is_empty());
    }

    #[test]
    fn a_fix_inside_a_domain_is_dropped() {
        let text = "Pick an oldad from frame.io. Don't publish";
        let kept = keep(
            text,
            vec![fix(text, "oldad", "old ad"), fix(text, "io", "i")],
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].replacement, "old ad");
    }

    #[test]
    fn adding_after_a_domain_stays() {
        let text = "see frame.io";
        assert_eq!(keep(text, vec![fix(text, "io", "io.")]).len(), 1);
        assert_eq!(
            keep(text, vec![fix(text, "frame.io", "frame.com")]).len(),
            0
        );
    }

    #[test]
    fn a_domain_may_be_capitalised_and_nothing_else_may() {
        let text = "from frame.io and AdCollection at https://frame.io/x";
        assert_eq!(
            keep(text, vec![fix(text, "frame.io and", "Frame.io and")]).len(),
            1
        );
        assert_eq!(
            keep(text, vec![fix(text, "AdCollection", "Adcollection")]).len(),
            0
        );
        assert_eq!(
            keep(
                text,
                vec![fix(text, "https://frame.io/x", "https://Frame.io/x")]
            )
            .len(),
            0
        );
    }
}
