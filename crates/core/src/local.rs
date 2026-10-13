//! The on-device pass: Harper, run on every pause in typing.
//!
//! Cheap enough to run on every pause - a paragraph takes milliseconds - so
//! the first underlines never wait for the network, and still useful with no
//! key and no connection.

use crate::fix::{Fix, Kind, Source};
use harper_core::linting::{LintGroup, LintKind, Linter, Suggestion};
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};
use std::collections::HashSet;

pub struct Local {
    linter: LintGroup,
    /// Lowercased words the user taught it. Matched against the flagged text
    /// rather than added to Harper's dictionary, so a word added in settings
    /// applies on the next pause without rebuilding the linter.
    known: HashSet<String>,
}

impl Local {
    pub fn new(dialect: Dialect) -> Self {
        Self {
            linter: LintGroup::new_curated(FstDictionary::curated(), dialect),
            known: HashSet::new(),
        }
    }

    pub fn set_known_words(&mut self, words: impl IntoIterator<Item = String>) {
        self.known = words.into_iter().map(|w| w.to_lowercase()).collect();
    }

    /// Fixes for the English sentences of `text`. Other languages are left
    /// to the model; see `lang`.
    ///
    /// Judged per sentence, not per paragraph: "This is cool and nice. Mais
    /// je ne pourrai pas faire l'honneur." is one line, read as English, and
    /// Harper offered "purr's" for "pourrai" and "longer" for "l'honneur".
    /// Harper still reads the whole paragraph, for its context; what it says
    /// about a sentence in another language is dropped.
    pub fn check(&mut self, text: &str) -> Vec<Fix> {
        let mut fixes = Vec::new();
        for (offset, paragraph) in crate::worker::paragraphs(text) {
            if !crate::lang::detect(paragraph).local_pass() && !mixed(paragraph) {
                continue;
            }
            let mut verdicts: Vec<((usize, usize), bool)> = Vec::new();
            for fix in self.check_english(paragraph) {
                let bounds = crate::fix::sentence(paragraph, fix.start);
                let english = match verdicts.iter().find(|(b, _)| *b == bounds) {
                    Some((_, english)) => *english,
                    None => {
                        let sentence: String = paragraph
                            .chars()
                            .skip(bounds.0)
                            .take(bounds.1 - bounds.0)
                            .collect();
                        let english = crate::lang::detect(&sentence).local_pass();
                        verdicts.push((bounds, english));
                        english
                    }
                };
                if english {
                    fixes.push(Fix {
                        start: fix.start + offset,
                        end: fix.end + offset,
                        ..fix
                    });
                }
            }
        }
        fixes
    }

    fn check_english(&mut self, text: &str) -> Vec<Fix> {
        let document = Document::new_plain_english_curated(text);
        let source = document.get_source();
        let mut fixes = Vec::new();
        for lint in self.linter.lint(&document) {
            let original: String = lint.span.get_content_string(source);
            if lint.lint_kind == LintKind::Spelling && self.known.contains(&original.to_lowercase())
            {
                continue;
            }
            // A lint with no edit is advice, and a card with nothing to accept
            // is a card that only interrupts. Harper's first suggestion is its
            // best; the rest stay out of a one-button card.
            let Some(suggestion) = lint.suggestions.first() else {
                continue;
            };
            let (start, end, replacement) = match suggestion {
                Suggestion::ReplaceWith(chars) => {
                    (lint.span.start, lint.span.end, chars.iter().collect())
                }
                Suggestion::InsertAfter(chars) => {
                    (lint.span.end, lint.span.end, chars.iter().collect())
                }
                Suggestion::Remove => (lint.span.start, lint.span.end, String::new()),
            };
            fixes.push(Fix {
                start,
                end,
                replacement,
                kind: kind_of(lint.lint_kind),
                message: plain(&lint.message),
                source: Source::Local,
            });
        }
        fixes
    }
}

/// Whether any sentence of a paragraph reads as English, so a mostly-French
/// paragraph with one English sentence still gets that sentence checked.
fn mixed(paragraph: &str) -> bool {
    let mut at = 0;
    let len = paragraph.chars().count();
    while at < len {
        let (start, end) = crate::fix::sentence(paragraph, at);
        let sentence: String = paragraph.chars().skip(start).take(end - start).collect();
        if crate::lang::detect(&sentence) == crate::lang::Language::English {
            return true;
        }
        at = end.max(at + 1);
    }
    false
}

fn kind_of(kind: LintKind) -> Kind {
    match kind {
        LintKind::Spelling => Kind::Spelling,
        LintKind::Typo
        | LintKind::Agreement
        | LintKind::Grammar
        | LintKind::Capitalization
        | LintKind::Punctuation
        | LintKind::BoundaryError
        | LintKind::Malapropism
        | LintKind::Eggcorn
        | LintKind::Repetition => Kind::Correctness,
        _ => Kind::Clarity,
    }
}

/// Harper's messages are Markdown; the card draws plain text.
fn plain(message: &str) -> String {
    message.replace(['`', '*'], "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fix::apply;

    #[test]
    fn finds_a_misspelling_and_fixes_it() {
        let mut local = Local::new(Dialect::American);
        let text = "This sentance has a typo.";
        let fixes = local.check(text);
        let typo = fixes
            .iter()
            .find(|f| f.original(text) == "sentance")
            .expect("Harper should flag 'sentance'");
        assert_eq!(typo.kind, Kind::Spelling);
        assert_eq!(
            apply(text, std::slice::from_ref(typo)),
            "This sentence has a typo."
        );
    }

    #[test]
    fn clean_text_has_nothing_to_say() {
        let mut local = Local::new(Dialect::American);
        assert!(local.check("The cat sat on the mat.").is_empty());
    }

    #[test]
    fn french_is_not_a_page_of_misspellings() {
        let mut local = Local::new(Dialect::American);
        let text = "Je pense que nous devrions partir demain matin avant le petit déjeuner.";
        assert!(local.check(text).is_empty());
    }

    #[test]
    fn english_paragraph_after_a_french_one_is_still_checked() {
        let mut local = Local::new(Dialect::American);
        let text = "Je pense que nous devrions partir demain matin avant le petit déjeuner.\nThis sentance has a typo in it today.";
        let fixes = local.check(text);
        assert!(
            fixes.iter().any(|f| f.original(text) == "sentance"),
            "{fixes:?}"
        );
    }

    #[test]
    fn french_sentence_after_english_on_one_line_is_left_alone() {
        let mut local = Local::new(Dialect::American);
        let text = "This is cool and nice. Mais je ne pourrai pas faire l'honneur.";
        let fixes = local.check(text);
        let french_start = text.find("Mais").unwrap();
        assert!(
            fixes.iter().all(|f| f.start < french_start),
            "{:?}",
            fixes.iter().map(|f| f.original(text)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn english_mistake_before_french_on_one_line_is_still_caught() {
        let mut local = Local::new(Dialect::American);
        let text = "This sentance is cool and nice. Mais je ne pourrai pas faire l'honneur.";
        let fixes = local.check(text);
        assert!(
            fixes.iter().any(|f| f.original(text) == "sentance"),
            "{fixes:?}"
        );
        let french_start = text.find("Mais").unwrap();
        assert!(fixes.iter().all(|f| f.start < french_start), "{fixes:?}");
    }

    #[test]
    fn a_known_word_is_not_a_misspelling() {
        let mut local = Local::new(Dialect::American);
        let text = "Gramly runs on Hyprland.";
        local.set_known_words(["gramly".into(), "Hyprland".into()]);
        let fixes = local.check(text);
        assert!(
            !fixes
                .iter()
                .any(|f| matches!(f.original(text).as_str(), "Gramly" | "Hyprland")),
            "{fixes:?}"
        );
    }
}
