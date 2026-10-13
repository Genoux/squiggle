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

    pub fn check(&mut self, text: &str) -> Vec<Fix> {
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
