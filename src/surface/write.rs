//! Writing accepted fixes into the field: directly when the app honours it,
//! as a paste over a confirmed selection when it does not. Shared by the
//! engine and by `squiggle lab`, which runs this same code against real text
//! fields to audit it.
//!
//! When in doubt nothing is written: every refusal says why, and the caller
//! logs it.

use super::{Field, Write};
use squiggle_core::{Fix, apply};
use std::time::{Duration, Instant};

/// Why a fix was not written.
pub type Refusal = &'static str;

/// How long an app gets to show a selection or a write. Zen (Gecko) applies
/// both a moment late; read back at once, a write looked refused and the fix
/// was pasted a second time.
pub const SETTLE: Duration = Duration::from_millis(300);

/// The clipboard side of a paste, in the order it must happen: `stage`
/// before anything is selected, so that nothing stands between the confirmed
/// selection and the keystroke. After a successful `stage` exactly one of
/// `send` and `unstage` follows, and either puts the user's clipboard back
/// (`send` after a delay, once the app has read it).
pub trait Paste {
    /// Saves the clipboard and puts `text` on it. Refuses when the clipboard
    /// cannot be saved whole: it could not be put back.
    fn stage(&self, text: &str) -> Result<(), Refusal>;
    /// Sends the paste keystroke. A refusal has already put the clipboard
    /// back.
    fn send(&self) -> Result<(), Refusal>;
    /// Puts the clipboard back without pasting.
    fn unstage(&self);
    /// Sends Backspace over the selection, touching no clipboard.
    fn delete_selection(&self) -> Result<(), Refusal>;
}

/// Whether `done` comes true within [`SETTLE`], asked every 15ms.
pub fn settle(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + SETTLE;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// The field as `read` finds it once it holds `expected`, or as it last
/// read after [`SETTLE`].
pub fn read_back(mut read: impl FnMut() -> Option<String>, expected: &str) -> Option<String> {
    let mut after = None;
    settle(|| {
        after = read();
        after.as_deref() == Some(expected)
    });
    after
}

/// What a write did, from the field before and after and what it should hold.
/// Only a field left exactly as it was counts as ignored: one that changed
/// into anything else did take a write, and a paste on top wrote it twice.
pub fn classify(before: &str, after: Option<&str>, expected: &str) -> Write {
    match after {
        Some(after) if after == expected => Write::Done,
        Some(after) if after == before => Write::Ignored,
        _ => Write::Unsafe,
    }
}

/// A range of the field being replaced.
struct Span<'a> {
    start: usize,
    length: usize,
    /// The text the range was found in.
    before: &'a str,
    replacement: &'a str,
}

/// One fix. Returns the text the field should now hold.
pub fn one<F: Field>(
    field: &F,
    text: &str,
    fix: &Fix,
    paste: &dyn Paste,
) -> Result<String, Refusal> {
    unchanged(field, text)?;
    let expected = apply(text, std::slice::from_ref(fix));
    let length = fix.end - fix.start;
    match field.replace(fix.start, length, &fix.replacement, text, &expected) {
        Write::Done => {}
        Write::Ignored => {
            let span = Span {
                start: fix.start,
                length,
                before: text,
                replacement: &fix.replacement,
            };
            paste_over(field, paste, &span)?;
        }
        Write::Unsafe => return Err("the write could not be confirmed"),
    }
    Ok(expected)
}

/// Several fixes at once, back to front so each write leaves the offsets of
/// the rest valid. If the app refuses direct writes, the span they cover is
/// pasted once, corrected - a paste per fix would race the clipboard.
/// Returns the text the field should now hold.
pub fn many<F: Field>(
    field: &F,
    text: &str,
    fixes: &[Fix],
    paste: &dyn Paste,
) -> Result<String, Refusal> {
    let mut fixes = fixes.to_vec();
    fixes.sort_by_key(|f| f.start);
    if fixes.is_empty() {
        return Ok(text.to_string());
    }
    unchanged(field, text)?;
    let corrected = apply(text, &fixes);
    if let Some(current) = write_each(field, text, &fixes)? {
        paste_span(field, paste, (text, &current, &corrected), &fixes)?;
    }
    Ok(corrected)
}

/// Writes each fix directly, last first. `None` when all were written;
/// otherwise the text as it stands where the app stopped taking writes.
fn write_each<F: Field>(field: &F, text: &str, fixes: &[Fix]) -> Result<Option<String>, Refusal> {
    let mut current = text.to_string();
    for fix in fixes.iter().rev() {
        let next = apply(&current, std::slice::from_ref(fix));
        let length = fix.end - fix.start;
        match field.replace(fix.start, length, &fix.replacement, &current, &next) {
            Write::Done => current = next,
            Write::Ignored => return Ok(Some(current)),
            Write::Unsafe => return Err("a write could not be confirmed"),
        }
    }
    Ok(None)
}

/// Pastes the span the fixes cover, corrected. Every change lies inside
/// `[lo, hi)`, so outside it `current`, `text` and `corrected` agree and
/// only the span's end moves.
fn paste_span<F: Field>(
    field: &F,
    paste: &dyn Paste,
    (text, current, corrected): (&str, &str, &str),
    fixes: &[Fix],
) -> Result<(), Refusal> {
    let lo = fixes.iter().map(|f| f.start).min().unwrap_or_default();
    let hi = fixes.iter().map(|f| f.end).max().unwrap_or_default();
    let len = |s: &str| s.chars().count() as isize;
    let current_hi = (hi as isize + len(current) - len(text)) as usize;
    let corrected_hi = (hi as isize + len(corrected) - len(text)) as usize;
    let replacement: String = corrected.chars().skip(lo).take(corrected_hi - lo).collect();
    let covered: String = current.chars().skip(lo).take(current_hi - lo).collect();
    // Pasted across a line, a web list re-split its items around the paste
    // and left a stray space (`squiggle lab`, "Web list"). The app groups by
    // sentence, which never crosses a line, so this only guards an app that
    // reports its lines some other way.
    if covered.contains('\n') || replacement.contains('\n') {
        return Err("the fixes span more than one line");
    }
    let span = Span {
        start: lo,
        length: current_hi - lo,
        before: current,
        replacement: &replacement,
    };
    paste_over(field, paste, &span)
}

/// Whether the field still holds `text`, the text the fixes were found in.
/// Read now rather than trusted from the last tick: offsets into text that
/// has moved on select the wrong words.
fn unchanged<F: Field>(field: &F, text: &str) -> Result<(), Refusal> {
    if field.value().as_deref() == Some(text) {
        Ok(())
    } else {
        Err("the text changed before the fix was applied")
    }
}

/// The paste fallback, only over a confirmed selection of text that is
/// still what it was: a paste replaces whatever is selected, and over the
/// wrong selection it replaced a whole paragraph with one word.
///
/// The clipboard is staged before the selection is made. Staged after, the
/// gap between a confirmed selection and the keystroke was as long as the
/// clipboard's round trip, and the field could move on inside it.
fn paste_over<F: Field>(field: &F, paste: &dyn Paste, span: &Span) -> Result<(), Refusal> {
    // Zen ignores a paste of empty text: the word stayed selected and the
    // next key typed replaced it. Backspace removes a selection everywhere.
    if span.replacement.is_empty() {
        // Over an empty selection, Backspace takes the character before it.
        if span.length == 0 {
            return Err("nothing to remove");
        }
        select_and_check(field, span).inspect_err(|_| field.deselect())?;
        return paste.delete_selection().inspect_err(|_| field.deselect());
    }
    paste.stage(span.replacement)?;
    if let Err(refusal) = select_and_check(field, span) {
        paste.unstage();
        field.deselect();
        return Err(refusal);
    }
    paste.send().inspect_err(|_| field.deselect())
}

fn select_and_check<F: Field>(field: &F, span: &Span) -> Result<(), Refusal> {
    if !field.select(span.start, span.length) {
        return Err("the app did not confirm the selection");
    }
    // A write that was slow rather than ignored may land while selecting.
    if field.value().as_deref() != Some(span.before) {
        return Err("the text changed while selecting");
    }
    if !field.still_selected() {
        return Err("the selection moved before the paste");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Rect;
    use squiggle_core::{Kind, Source};
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<String>>>;

    struct Fake {
        text: RefCell<String>,
        modes: RefCell<VecDeque<Write>>,
        select_ok: bool,
        still_selected: bool,
        text_after_select: Option<String>,
        replaced: RefCell<Vec<(usize, usize, String)>>,
        selected: RefCell<Vec<(usize, usize)>>,
        deselects: Cell<u32>,
        log: Log,
    }

    impl Fake {
        fn holding(text: &str, log: &Log) -> Self {
            Self {
                text: RefCell::new(text.into()),
                modes: RefCell::new(VecDeque::new()),
                select_ok: true,
                still_selected: true,
                text_after_select: None,
                replaced: RefCell::new(Vec::new()),
                selected: RefCell::new(Vec::new()),
                deselects: Cell::new(0),
                log: Rc::clone(log),
            }
        }

        fn answering(self, modes: &[Write]) -> Self {
            *self.modes.borrow_mut() = modes.iter().copied().collect();
            self
        }
    }

    impl PartialEq for Fake {
        fn eq(&self, other: &Self) -> bool {
            std::ptr::eq(self, other)
        }
    }

    impl Field for Fake {
        type Handle = ();

        fn handle(&self) {}

        fn place(&self) -> squiggle_core::off::Place {
            squiggle_core::off::Place::default()
        }

        fn value(&self) -> Option<String> {
            Some(self.text.borrow().clone())
        }

        fn visible_frame(&self) -> Option<Rect> {
            None
        }

        fn bounds(&self, _start: usize, _len: usize) -> Option<Rect> {
            None
        }

        fn select(&self, start: usize, len: usize) -> bool {
            self.log.borrow_mut().push("select".into());
            self.selected.borrow_mut().push((start, len));
            if let Some(text) = &self.text_after_select {
                *self.text.borrow_mut() = text.clone();
            }
            self.select_ok
        }

        fn still_selected(&self) -> bool {
            self.still_selected
        }

        fn deselect(&self) {
            self.deselects.set(self.deselects.get() + 1);
        }

        fn replace(&self, start: usize, len: usize, text: &str, _: &str, _: &str) -> Write {
            self.replaced.borrow_mut().push((start, len, text.into()));
            let mode = self.modes.borrow_mut().pop_front().unwrap_or(Write::Done);
            if mode == Write::Done {
                let chars: Vec<char> = self.text.borrow().chars().collect();
                let rewritten: String = chars[..start]
                    .iter()
                    .chain(text.chars().collect::<Vec<_>>().iter())
                    .chain(chars[start + len..].iter())
                    .collect();
                *self.text.borrow_mut() = rewritten;
            }
            mode
        }
    }

    struct FakePaste {
        log: Log,
        stage: Result<(), Refusal>,
        send: Result<(), Refusal>,
    }

    impl FakePaste {
        fn new(log: &Log) -> Self {
            Self {
                log: Rc::clone(log),
                stage: Ok(()),
                send: Ok(()),
            }
        }
    }

    impl Paste for FakePaste {
        fn stage(&self, text: &str) -> Result<(), Refusal> {
            self.log.borrow_mut().push(format!("stage {text}"));
            self.stage
        }

        fn send(&self) -> Result<(), Refusal> {
            self.log.borrow_mut().push("send".into());
            self.send
        }

        fn unstage(&self) {
            self.log.borrow_mut().push("unstage".into());
        }

        fn delete_selection(&self) -> Result<(), Refusal> {
            self.log.borrow_mut().push("delete".into());
            Ok(())
        }
    }

    fn fix(start: usize, end: usize, replacement: &str) -> Fix {
        Fix {
            start,
            end,
            replacement: replacement.into(),
            kind: Kind::Spelling,
            message: String::new(),
            source: Source::Local,
        }
    }

    fn log() -> Log {
        Rc::default()
    }

    fn events(log: &Log) -> Vec<String> {
        log.borrow().clone()
    }

    #[test]
    fn one_writes_directly_when_the_app_honours_it() {
        let log = log();
        let field = Fake::holding("teh cat", &log);
        let result = one(&field, "teh cat", &fix(0, 3, "the"), &FakePaste::new(&log));
        assert_eq!(result, Ok("the cat".to_string()));
        assert_eq!(*field.replaced.borrow(), [(0, 3, "the".to_string())]);
        assert!(events(&log).is_empty());
    }

    #[test]
    fn one_stages_the_clipboard_before_it_selects_and_sends_at_once() {
        let log = log();
        let field = Fake::holding("teh cat", &log).answering(&[Write::Ignored]);
        let result = one(&field, "teh cat", &fix(0, 3, "the"), &FakePaste::new(&log));
        assert_eq!(result, Ok("the cat".to_string()));
        assert_eq!(*field.selected.borrow(), [(0, 3)]);
        assert_eq!(events(&log), ["stage the", "select", "send"]);
        assert_eq!(field.deselects.get(), 0);
    }

    #[test]
    fn one_refuses_when_the_write_cannot_be_confirmed() {
        let log = log();
        let field = Fake::holding("teh cat", &log).answering(&[Write::Unsafe]);
        let result = one(&field, "teh cat", &fix(0, 3, "the"), &FakePaste::new(&log));
        assert_eq!(result, Err("the write could not be confirmed"));
        assert!(events(&log).is_empty());
    }

    #[test]
    fn one_refuses_text_that_moved_on() {
        let log = log();
        let field = Fake::holding("teh cat sat", &log);
        let result = one(&field, "teh cat", &fix(0, 3, "the"), &FakePaste::new(&log));
        assert_eq!(result, Err("the text changed before the fix was applied"));
        assert!(field.replaced.borrow().is_empty());
    }

    #[test]
    fn a_clipboard_that_cannot_be_staged_selects_nothing() {
        let log = log();
        let field = Fake::holding("teh cat", &log).answering(&[Write::Ignored]);
        let paste = FakePaste {
            stage: Err("the clipboard could not be saved"),
            ..FakePaste::new(&log)
        };
        let result = one(&field, "teh cat", &fix(0, 3, "the"), &paste);
        assert_eq!(result, Err("the clipboard could not be saved"));
        assert_eq!(events(&log), ["stage the"]);
        assert!(field.selected.borrow().is_empty());
    }

    #[test]
    fn one_does_not_paste_over_an_unconfirmed_selection() {
        let log = log();
        let mut field = Fake::holding("teh cat", &log).answering(&[Write::Ignored]);
        field.select_ok = false;
        let result = one(&field, "teh cat", &fix(0, 3, "the"), &FakePaste::new(&log));
        assert_eq!(result, Err("the app did not confirm the selection"));
        assert_eq!(events(&log), ["stage the", "select", "unstage"]);
        assert_eq!(field.deselects.get(), 1);
    }

    #[test]
    fn one_does_not_paste_when_the_text_changes_while_selecting() {
        let log = log();
        let mut field = Fake::holding("teh cat", &log).answering(&[Write::Ignored]);
        field.text_after_select = Some("the cat".into());
        let result = one(&field, "teh cat", &fix(0, 3, "the"), &FakePaste::new(&log));
        assert_eq!(result, Err("the text changed while selecting"));
        assert_eq!(events(&log), ["stage the", "select", "unstage"]);
        assert_eq!(field.deselects.get(), 1);
    }

    #[test]
    fn one_does_not_paste_when_the_selection_moved_before_the_keystroke() {
        let log = log();
        let mut field = Fake::holding("teh cat", &log).answering(&[Write::Ignored]);
        field.still_selected = false;
        let result = one(&field, "teh cat", &fix(0, 3, "the"), &FakePaste::new(&log));
        assert_eq!(result, Err("the selection moved before the paste"));
        assert_eq!(events(&log), ["stage the", "select", "unstage"]);
        assert_eq!(field.deselects.get(), 1);
    }

    #[test]
    fn a_refused_keystroke_leaves_the_field_as_found() {
        let log = log();
        let field = Fake::holding("teh cat", &log).answering(&[Write::Ignored]);
        let paste = FakePaste {
            send: Err("the window changed before the paste"),
            ..FakePaste::new(&log)
        };
        let result = one(&field, "teh cat", &fix(0, 3, "the"), &paste);
        assert_eq!(result, Err("the window changed before the paste"));
        assert_eq!(field.deselects.get(), 1);
        assert_eq!(events(&log), ["stage the", "select", "send"]);
    }

    #[test]
    fn a_removal_is_deleted_over_the_selection_not_pasted() {
        let log = log();
        let field = Fake::holding("a big cat", &log).answering(&[Write::Ignored]);
        let result = one(&field, "a big cat", &fix(1, 5, ""), &FakePaste::new(&log));
        assert_eq!(result, Ok("a cat".to_string()));
        assert_eq!(*field.selected.borrow(), [(1, 4)]);
        assert_eq!(events(&log), ["select", "delete"]);
        assert_eq!(field.deselects.get(), 0);
    }

    #[test]
    fn a_removal_over_an_unconfirmed_selection_deletes_nothing() {
        let log = log();
        let mut field = Fake::holding("a big cat", &log).answering(&[Write::Ignored]);
        field.select_ok = false;
        let result = one(&field, "a big cat", &fix(1, 5, ""), &FakePaste::new(&log));
        assert_eq!(result, Err("the app did not confirm the selection"));
        assert_eq!(events(&log), ["select"]);
        assert_eq!(field.deselects.get(), 1);
    }

    #[test]
    fn an_empty_removal_sends_no_backspace() {
        let log = log();
        let field = Fake::holding("a big cat", &log).answering(&[Write::Ignored]);
        let result = one(&field, "a big cat", &fix(1, 1, ""), &FakePaste::new(&log));
        assert_eq!(result, Err("nothing to remove"));
        assert!(events(&log).is_empty());
    }

    #[test]
    fn many_writes_back_to_front() {
        let text = "a teh and wrold";
        let log = log();
        let field = Fake::holding(text, &log);
        let fixes = [fix(10, 15, "world"), fix(2, 5, "the")];
        let result = many(&field, text, &fixes, &FakePaste::new(&log));
        assert_eq!(result, Ok("a the and world".to_string()));
        assert_eq!(
            *field.replaced.borrow(),
            [(10, 5, "world".to_string()), (2, 3, "the".to_string())]
        );
        assert_eq!(*field.text.borrow(), "a the and world");
        assert!(events(&log).is_empty());
    }

    #[test]
    fn many_pastes_the_span_once_when_the_first_write_is_ignored() {
        let text = "i went to paris today";
        let log = log();
        let field = Fake::holding(text, &log).answering(&[Write::Ignored]);
        let fixes = [fix(0, 1, "I"), fix(10, 15, "Paris")];
        let result = many(&field, text, &fixes, &FakePaste::new(&log));
        assert_eq!(result, Ok("I went to Paris today".to_string()));
        assert_eq!(*field.selected.borrow(), [(0, 15)]);
        assert_eq!(events(&log), ["stage I went to Paris", "select", "send"]);
    }

    #[test]
    fn many_pastes_the_span_as_it_stands_after_direct_writes() {
        let text = "a teh and wrold";
        let log = log();
        let field = Fake::holding(text, &log).answering(&[Write::Done, Write::Ignored]);
        let fixes = [fix(2, 5, "them"), fix(10, 15, "world")];
        let result = many(&field, text, &fixes, &FakePaste::new(&log));
        assert_eq!(result, Ok("a them and world".to_string()));
        assert_eq!(*field.selected.borrow(), [(2, 13)]);
        assert_eq!(events(&log), ["stage them and world", "select", "send"]);
    }

    #[test]
    fn many_refuses_a_span_that_crosses_a_line() {
        let text = "teh cat\nsat on teh mat";
        let log = log();
        let field = Fake::holding(text, &log).answering(&[Write::Ignored]);
        let fixes = [fix(0, 3, "the"), fix(15, 18, "the")];
        let result = many(&field, text, &fixes, &FakePaste::new(&log));
        assert_eq!(result, Err("the fixes span more than one line"));
        assert!(field.selected.borrow().is_empty());
        assert!(events(&log).is_empty());
    }

    #[test]
    fn many_with_nothing_to_fix_writes_nothing() {
        let log = log();
        let field = Fake::holding("fine", &log);
        assert_eq!(
            many(&field, "fine", &[], &FakePaste::new(&log)),
            Ok("fine".to_string())
        );
        assert!(field.replaced.borrow().is_empty());
    }

    #[test]
    fn a_field_that_reads_as_expected_is_done() {
        assert_eq!(classify("teh cat", Some("the cat"), "the cat"), Write::Done);
    }

    #[test]
    fn a_field_exactly_as_it_was_is_ignored() {
        assert_eq!(
            classify("teh cat", Some("teh cat"), "the cat"),
            Write::Ignored
        );
    }

    #[test]
    fn a_field_changed_into_something_else_is_unsafe() {
        assert_eq!(classify("teh cat", Some("the"), "the cat"), Write::Unsafe);
        assert_eq!(
            classify("teh cat", Some("teh cats"), "the cat"),
            Write::Unsafe
        );
    }

    #[test]
    fn a_field_that_cannot_be_read_back_is_unsafe() {
        assert_eq!(classify("teh cat", None, "the cat"), Write::Unsafe);
    }

    #[test]
    fn expected_wins_when_a_fix_changes_nothing() {
        assert_eq!(classify("same", Some("same"), "same"), Write::Done);
    }

    #[test]
    fn settling_waits_for_a_late_answer() {
        let mut asks = 0;
        assert!(settle(|| {
            asks += 1;
            asks == 3
        }));
        assert_eq!(asks, 3);
    }

    #[test]
    fn settling_gives_up_after_the_deadline() {
        let started = Instant::now();
        assert!(!settle(|| false));
        assert!(started.elapsed() >= SETTLE);
    }

    #[test]
    fn reading_back_returns_the_text_once_it_is_expected() {
        let mut reads = ["teh", "teh", "the"].into_iter();
        let after = read_back(|| reads.next().map(String::from), "the");
        assert_eq!(after.as_deref(), Some("the"));
    }

    #[test]
    fn reading_back_returns_the_last_read_when_it_never_comes() {
        let after = read_back(|| Some("teh".into()), "the");
        assert_eq!(after.as_deref(), Some("teh"));
    }
}
