//! What a suggestion card says, as data. Each platform decides how it looks.

use gramly_core::diff::{Piece, pieces};
use gramly_core::{Fix, Kind, Source, improve};

/// What clicking a button of the card does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Accept,
    Dismiss,
    Learn,
    AcceptSentence,
    DismissSentence,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Accept => "Accept",
            Action::Dismiss | Action::DismissSentence => "Dismiss",
            Action::Learn => "Add to dictionary",
            Action::AcceptSentence => "Correct the sentence",
        }
    }

    /// The name `GRAMLY_STATE` carries, which `gramly lab --e2e` reads.
    pub fn name(self) -> &'static str {
        match self {
            Action::Accept => "accept:",
            Action::Dismiss => "dismiss:",
            Action::Learn => "learn:",
            Action::AcceptSentence => "acceptSentence:",
            Action::DismissSentence => "dismissSentence:",
        }
    }
}

/// The card for the hovered fix: its sentence with the change in place, then
/// its buttons. One fix or several, a correction or clearer wording, the
/// layout is the same; only the title and the buttons change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub title: String,
    /// Colours the marker beside the title and what the change takes out:
    /// clearer wording is advice, anything else a mistake.
    pub kind: Kind,
    pub bits: Vec<Bit>,
    /// The first is the main one.
    pub actions: Vec<Action>,
}

/// A piece of the card's sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bit {
    Same(String),
    /// What goes, struck.
    Gone(String),
    /// What comes in.
    New(String),
}

/// `group` is every fix in the hovered fix's sentence, by index, in order.
pub fn card(index: usize, group: &[(usize, Fix)], text: &str) -> Option<Card> {
    let (_, fix) = group.iter().find(|(i, _)| *i == index)?;
    // Clarity first: its card is never part of "Correct the sentence".
    let (title, kind, members, actions) = if fix.kind == Kind::Clarity {
        (
            improve::TITLE.to_string(),
            Kind::Clarity,
            vec![fix],
            vec![Action::Accept, Action::Dismiss],
        )
    } else if group.len() > 1 {
        (
            Action::AcceptSentence.label().to_string(),
            Kind::Correctness,
            group.iter().map(|(_, fix)| fix).collect(),
            vec![Action::AcceptSentence, Action::DismissSentence],
        )
    } else {
        let mut actions = vec![Action::Accept, Action::Dismiss];
        if fix.kind == Kind::Spelling {
            actions.push(Action::Learn);
        }
        (title(fix, text), fix.kind, vec![fix], actions)
    };
    Some(Card {
        title,
        kind,
        bits: snippet(text, &members),
        actions,
    })
}

/// The title. The model's titles are written for this; Harper's messages are
/// written for a list ("Did you mean to spell recieve this way?") and read
/// oddly as a title, so local fixes get a plain one.
fn title(fix: &Fix, text: &str) -> String {
    let original = fix.original(text);
    // A doubled space's card: the change is invisible, the title says it.
    if !original.is_empty()
        && original.chars().all(char::is_whitespace)
        && fix.replacement.chars().all(char::is_whitespace)
    {
        return "Correct your spacing".into();
    }
    if fix.source == Source::Ai && !fix.message.is_empty() {
        return fix.message.clone();
    }
    match fix.kind {
        Kind::Spelling => "Correct your spelling",
        Kind::Correctness => "Correct your grammar",
        Kind::Clarity => improve::TITLE,
    }
    .to_string()
}

/// Words of context kept on each side of the change.
pub const CONTEXT: usize = 6;
/// A space that is the whole change, shown so the card is never blank.
const SPACE: &str = "\u{2423}";

/// The sentence the fixes are in, with each change in place: the struck
/// original and what replaces it. Cut to `CONTEXT` words either side, so a
/// long sentence does not make a long card.
///
/// It replaced a lone replacement word. Out of its sentence, "it's" did not
/// say why, and a removal had to be shown as what was left ("me the" for
/// "send me the the link").
fn snippet(text: &str, fixes: &[&Fix]) -> Vec<Bit> {
    let chars: Vec<char> = text.chars().collect();
    let mut fixes: Vec<&Fix> = fixes
        .iter()
        .copied()
        .filter(|f| f.start <= f.end && f.end <= chars.len())
        .collect();
    fixes.sort_by_key(|f| f.start);
    let Some(first) = fixes.first().map(|f| f.start) else {
        return Vec::new();
    };
    let last = fixes.iter().map(|f| f.end).max().unwrap_or(first);
    let (from, to) = gramly_core::sentence(text, first);
    let (from, to) = (from.min(first), to.max(last).min(chars.len()));

    let mut out = vec![Bit::Same(lead(&chars[from..first]))];
    let mut cursor = first;
    for fix in fixes {
        if fix.start < cursor {
            continue;
        }
        out.push(Bit::Same(chars[cursor..fix.start].iter().collect()));
        let original: String = chars[fix.start..fix.end].iter().collect();
        for piece in bridged(pieces(&original, &fix.replacement)) {
            match piece {
                Piece::Same(same) => out.push(Bit::Same(same)),
                Piece::Changed { was, now } => change(&mut out, &was, &now),
            }
        }
        cursor = fix.end;
    }
    out.push(Bit::Same(trail(&chars[cursor..to.max(cursor)])));

    // Merged and flattened: a sentence on a card is one line of thought. An
    // editor's inline objects drew as boxed "OBJ" in the card's font.
    let shown = |s: String| -> String {
        s.chars()
            .filter(|c| !gramly_core::protect::invisible(*c))
            .map(|c| if c == '\n' { ' ' } else { c })
            .collect()
    };
    let mut tidy: Vec<Bit> = Vec::new();
    for bit in out {
        match (tidy.last_mut(), bit) {
            (Some(Bit::Same(prev)), Bit::Same(s)) => prev.push_str(&shown(s)),
            (_, Bit::Same(s)) => tidy.push(Bit::Same(shown(s))),
            (_, other) => tidy.push(other),
        }
    }
    tidy.retain(|bit| !matches!(bit, Bit::Same(s) if s.is_empty()));
    tidy
}

/// Two changes with one word kept between them, as one change. A rewrite
/// shown word by word left "we" standing alone between two struck phrases
/// ("~~it would maybe be better if~~ we ~~could try to~~ should"), which
/// reads worse than the phrase struck and the new one beside it.
fn bridged(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for piece in pieces {
        if let Piece::Changed { was, now } = &piece
            && let [.., Piece::Changed { .. }, Piece::Same(kept)] = out.as_slice()
            && !kept.trim().is_empty()
            && !kept.trim().contains(char::is_whitespace)
        {
            let kept = kept.clone();
            out.pop();
            if let Some(Piece::Changed { was: w, now: n }) = out.last_mut() {
                w.push_str(&kept);
                w.push_str(was);
                n.push_str(&kept);
                n.push_str(now);
                continue;
            }
        }
        out.push(piece);
    }
    out
}

/// One changed stretch as bits. Spaces around a word stay plain text; a
/// change of spaces alone is shown with `SPACE`.
fn change(out: &mut Vec<Bit>, was: &str, now: &str) {
    let blank = |s: &str| s.trim().is_empty();
    if blank(was) && blank(now) {
        let (gone, came) = (was.chars().count(), now.chars().count());
        if gone > came {
            // Spaced either side, so the mark does not cling to a word.
            out.push(Bit::Same(now.into()));
            out.push(Bit::Gone(SPACE.repeat(gone - came)));
            out.push(Bit::Same(" ".into()));
        } else {
            out.push(Bit::Same(was.into()));
            out.push(Bit::New(SPACE.repeat(came - gone)));
        }
        return;
    }
    let edges = |s: &str| {
        let start = &s[..s.len() - s.trim_start().len()];
        let end = &s[s.trim_end().len()..];
        (start.to_string(), end.to_string())
    };
    // The spaces the sentence ends up with.
    let (lead, tail) = if blank(now) { edges(was) } else { edges(now) };
    out.push(Bit::Same(lead));
    if !blank(was) {
        out.push(Bit::Gone(was.trim().into()));
    }
    if !blank(now) {
        if !blank(was) {
            out.push(Bit::Same(" ".into()));
        }
        out.push(Bit::New(now.trim().into()));
    }
    out.push(Bit::Same(tail));
}

fn word_starts(chars: &[char]) -> Vec<usize> {
    (0..chars.len())
        .filter(|&i| !chars[i].is_whitespace() && (i == 0 || chars[i - 1].is_whitespace()))
        .collect()
}

/// The last `CONTEXT` words before the change.
fn lead(chars: &[char]) -> String {
    let starts = word_starts(chars);
    if starts.len() <= CONTEXT {
        return chars.iter().collect();
    }
    let cut = starts[starts.len() - CONTEXT];
    format!("…{}", chars[cut..].iter().collect::<String>())
}

/// The first `CONTEXT` words after the change.
fn trail(chars: &[char]) -> String {
    let starts = word_starts(chars);
    if starts.len() <= CONTEXT {
        return chars.iter().collect();
    }
    let cut = starts[CONTEXT];
    let kept: String = chars[..cut].iter().collect();
    format!("{}…", kept.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(text: &str, gone: &str, replacement: &str) -> Fix {
        let byte = text.rfind(gone).unwrap();
        let start = text[..byte].chars().count();
        Fix {
            start,
            end: start + gone.chars().count(),
            replacement: replacement.into(),
            kind: Kind::Correctness,
            message: String::new(),
            source: Source::Ai,
        }
    }

    fn same(s: &str) -> Bit {
        Bit::Same(s.into())
    }

    #[test]
    fn a_word_is_shown_in_its_sentence() {
        let text = "Did you recieve the photos?";
        let f = fix(text, "recieve", "receive");
        assert_eq!(
            snippet(text, &[&f]),
            vec![
                same("Did you "),
                Bit::Gone("recieve".into()),
                same(" "),
                Bit::New("receive".into()),
                same(" the photos?"),
            ]
        );
    }

    #[test]
    fn objects_that_draw_nothing_are_left_out() {
        let text = "thi is a lest\u{FFFC}\u{FFFC}";
        let f = fix(text, "lest", "test");
        assert_eq!(
            snippet(text, &[&f]),
            vec![
                same("thi is a "),
                Bit::Gone("lest".into()),
                same(" "),
                Bit::New("test".into()),
            ]
        );
    }

    #[test]
    fn a_removal_is_struck_where_it_was() {
        let text = "Can you send me the the link for the frame.io draft when you have a sec?";
        let byte = text.find("the the").unwrap() + 4;
        let start = text[..byte].chars().count();
        let f = Fix {
            start,
            end: start + 4,
            replacement: String::new(),
            kind: Kind::Correctness,
            message: String::new(),
            source: Source::Ai,
        };
        assert_eq!(
            snippet(text, &[&f]),
            vec![
                same("Can you send me the "),
                Bit::Gone("the".into()),
                same(" link for the frame.io draft when…"),
            ]
        );
    }

    #[test]
    fn a_long_sentence_is_cut_around_the_change() {
        let text = "One two three four five six seven eight nine tehn eleven twelve thirteen fourteen fifteen sixteen seventeen.";
        let f = fix(text, "tehn", "ten");
        let bits = snippet(text, &[&f]);
        assert_eq!(
            bits.first(),
            Some(&same("…four five six seven eight nine "))
        );
        assert_eq!(
            bits.last(),
            Some(&same(" eleven twelve thirteen fourteen fifteen sixteen…"))
        );
    }

    #[test]
    fn a_change_of_spaces_alone_is_visible() {
        let text = "yes  was";
        let f = fix(text, "  ", " ");
        assert_eq!(
            snippet(text, &[&f]),
            vec![same("yes "), Bit::Gone(SPACE.into()), same(" was")]
        );
    }

    #[test]
    fn a_rewrite_is_not_shown_word_by_word() {
        let text = "I think it would maybe be better if we could try to move it.";
        let mut f = fix(
            text,
            "it would maybe be better if we could try to",
            "we should",
        );
        f.kind = Kind::Clarity;
        assert_eq!(
            snippet(text, &[&f]),
            vec![
                same("I think "),
                Bit::Gone("it would maybe be better if we could try to".into()),
                same(" "),
                Bit::New("we should".into()),
                same(" move it."),
            ]
        );
    }

    #[test]
    fn several_fixes_share_one_sentence() {
        let text = "i went to paris on monday and it was great.";
        let fixes = [
            fix(text, "i went", "I went"),
            fix(text, "paris", "Paris"),
            fix(text, "monday", "Monday"),
        ];
        let refs: Vec<&Fix> = fixes.iter().collect();
        let bits = snippet(text, &refs);
        let news: Vec<&Bit> = bits.iter().filter(|b| matches!(b, Bit::New(_))).collect();
        assert_eq!(
            news,
            vec![
                &Bit::New("I".into()),
                &Bit::New("Paris".into()),
                &Bit::New("Monday".into())
            ]
        );
        assert_eq!(bits.last(), Some(&same(" and it was great.")));
    }

    fn card_fix(start: usize, end: usize, replacement: &str, kind: Kind) -> Fix {
        Fix {
            start,
            end,
            replacement: replacement.into(),
            kind,
            message: String::new(),
            source: Source::Local,
        }
    }

    fn plain_card(text: &str, fix: Fix) -> Card {
        card(0, &[(0, fix)], text).expect("a card")
    }

    #[test]
    fn the_models_message_is_the_title() {
        let mut model = card_fix(0, 3, "the", Kind::Correctness);
        model.source = Source::Ai;
        model.message = "Use the definite article".into();
        assert_eq!(
            plain_card("teh cat", model).title,
            "Use the definite article"
        );
    }

    #[test]
    fn local_fixes_get_a_plain_title_by_kind() {
        let titled = |kind| {
            let mut local = card_fix(0, 3, "the", kind);
            local.message = "Did you mean to spell teh this way?".into();
            plain_card("teh cat", local).title
        };
        assert_eq!(titled(Kind::Spelling), "Correct your spelling");
        assert_eq!(titled(Kind::Correctness), "Correct your grammar");
    }

    #[test]
    fn a_model_fix_without_a_message_gets_the_plain_title() {
        let mut model = card_fix(0, 3, "the", Kind::Correctness);
        model.source = Source::Ai;
        assert_eq!(plain_card("teh cat", model).title, "Correct your grammar");
    }

    #[test]
    fn removed_spaces_are_a_spacing_card() {
        let card = plain_card("a  b", card_fix(1, 3, " ", Kind::Correctness));
        assert_eq!(card.title, "Correct your spacing");
    }

    #[test]
    fn spaces_turned_into_a_comma_are_not_spacing() {
        let card = plain_card("yes  was", card_fix(3, 5, ", ", Kind::Correctness));
        assert_eq!(card.title, "Correct your grammar");
    }

    #[test]
    fn only_a_spelling_offers_to_learn_the_word() {
        let actions = |kind| plain_card("teh cat", card_fix(0, 3, "the", kind)).actions;
        assert_eq!(
            actions(Kind::Spelling),
            [Action::Accept, Action::Dismiss, Action::Learn]
        );
        assert_eq!(
            actions(Kind::Correctness),
            [Action::Accept, Action::Dismiss]
        );
    }

    #[test]
    fn clearer_wording_is_alone_with_its_own_title() {
        let text = "Can you think for now about it. Then go.";
        let group = [
            (0, card_fix(8, 30, "about it for now", Kind::Clarity)),
            (1, card_fix(32, 36, "Go", Kind::Correctness)),
        ];
        let card = card(0, &group, text).expect("a card");
        assert_eq!(card.title, improve::TITLE);
        assert_eq!(card.kind, Kind::Clarity);
        assert_eq!(card.actions, [Action::Accept, Action::Dismiss]);
    }

    #[test]
    fn two_fixes_in_a_sentence_make_the_sentence_card() {
        let text = "she go to paris.";
        let group = [
            (0, card_fix(4, 6, "goes", Kind::Correctness)),
            (1, card_fix(10, 15, "Paris", Kind::Spelling)),
        ];
        let card = card(1, &group, text).expect("a card");
        assert_eq!(card.title, "Correct the sentence");
        assert_eq!(
            card.actions,
            [Action::AcceptSentence, Action::DismissSentence]
        );
    }

    #[test]
    fn no_card_for_an_index_outside_the_group() {
        assert_eq!(
            card(3, &[(0, card_fix(0, 3, "the", Kind::Spelling))], "teh"),
            None
        );
    }

    #[test]
    fn buttons_say_what_clicking_them_does() {
        assert_eq!(Action::Dismiss.label(), "Dismiss");
        assert_eq!(Action::DismissSentence.label(), "Dismiss");
        assert_eq!(Action::Learn.label(), "Add to dictionary");
        assert_eq!(Action::AcceptSentence.label(), "Correct the sentence");
    }

    #[test]
    fn actions_are_named_as_the_lab_reads_them() {
        assert_eq!(Action::Accept.name(), "accept:");
        assert_eq!(Action::AcceptSentence.name(), "acceptSentence:");
        assert_eq!(Action::Learn.name(), "learn:");
    }
}
