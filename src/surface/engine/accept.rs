//! Applying fixes to the field, and checking afterwards that the field holds
//! what it should.

use super::{ACCEPTED_FOR, Engine, dismissal};
use crate::surface::write::{self, Refusal};
use crate::surface::{Host, Overlay, Rect};
use squiggle_core::Fix;
use std::time::{Duration, Instant};

/// How long an accepted fix may still be on its way into the field. A second
/// accept inside it is made against text the first has already moved: two
/// were seen to land on each other.
const PENDING: Duration = Duration::from_secs(1);

const ALREADY_ACCEPTED: Refusal = "already accepted; the app has not shown the change yet";

pub(super) fn accept_pending(accepted_at: Option<Instant>, now: Instant) -> bool {
    accepted_at.is_some_and(|at| now.saturating_duration_since(at) < PENDING)
}

/// Whether `fix`'s range of `text` no longer reads the words its card showed.
pub(super) fn words_moved(text: &str, fix: &Fix, words: &str) -> bool {
    fix.original(text) != words
}

impl<H: Host, O: Overlay> Engine<H, O> {
    fn accept_in_flight(&self) -> bool {
        let accepted_at = self.expecting.as_ref().map(|(_, _, at)| *at);
        accept_pending(accepted_at, Instant::now())
    }

    /// Starts the glow on the words about to be accepted.
    pub(super) fn flash(&mut self, indices: &[usize]) {
        let rects: Vec<Rect> = self
            .words
            .iter()
            .filter(|(i, _)| indices.contains(i))
            .map(|(_, rect)| *rect)
            .collect();
        if !rects.is_empty() {
            self.flashing = Some((rects, Instant::now()));
        }
    }

    /// One fix, through Accessibility if the app honours it and as a paste over a
    /// selection if it does not.
    ///
    /// A card that outlived its fix (`ORPHAN`) is accepted only while its
    /// range still reads the words it showed: offsets of text that moved on
    /// select other words.
    pub(super) fn accept_fix(&mut self, fix: &Fix) {
        if self.accept_in_flight() {
            return;
        }
        let moved = self
            .card_words
            .as_deref()
            .is_some_and(|words| words_moved(&self.text, fix, words));
        self.hide_card();
        if moved {
            return self.refused("the card's words moved");
        }
        if self.was_accepted(fix) {
            return self.refused(ALREADY_ACCEPTED);
        }
        let fix = fix.clone();
        let Some(field) = &self.field else {
            return;
        };
        let expected = match write::one(field, &self.text, &fix, self.host.clipboard()) {
            Ok(expected) => expected,
            Err(why) => return self.refused(why),
        };
        self.remember(std::slice::from_ref(&fix));
        self.expecting = Some((self.text.clone(), expected, Instant::now()));
    }

    /// Several fixes at once, back to front so each write leaves the offsets of
    /// the rest valid. If the app refuses direct writes, the span they cover is
    /// pasted once, corrected, over a selection of the same span - a paste per
    /// fix would race the clipboard, and a select-all would reformat text the
    /// user never asked to touch.
    pub(super) fn accept_fixes(&mut self, indices: &[usize]) {
        if self.field.is_none() || self.accept_in_flight() {
            return;
        }
        let fixes: Vec<Fix> = indices
            .iter()
            .filter_map(|i| self.fixes.get(*i).cloned())
            .collect();
        if fixes.is_empty() {
            return;
        }
        self.hide_card();
        if fixes.iter().any(|fix| self.was_accepted(fix)) {
            return self.refused(ALREADY_ACCEPTED);
        }
        let Some(field) = &self.field else {
            return;
        };
        let text = self.text.clone();
        let corrected = match write::many(field, &text, &fixes, self.host.clipboard()) {
            Ok(corrected) => corrected,
            Err(why) => return self.refused(why),
        };
        self.remember(&fixes);
        self.expecting = Some((text, corrected, Instant::now()));
    }

    /// After an accepted fix the field should read as expected within a second.
    /// A correction that ate the user's text is the worst thing Squiggle can do:
    /// a field that changed and is still `LOST` or more characters shorter than
    /// both what it held and what it should hold, once the app has had
    /// `SETTLED` to finish the paste, is taken back with ⌘Z. A sentence replaced
    /// by one word happened twice; typing in that second cannot remove that
    /// much, and a paste that is only slow leaves the field as it was. Any
    /// other mismatch is logged with its size.
    pub(super) fn audit(&mut self) {
        const LOST: usize = 10;
        const SETTLED: Duration = Duration::from_millis(250);
        let Some((before, expected, at)) = &self.expecting else {
            return;
        };
        if self.text == *expected {
            self.expecting = None;
            return;
        }
        let (got, want) = (self.text.chars().count(), expected.chars().count());
        let had = before.chars().count();
        let wrote = self.text != *before;
        if at.elapsed() >= SETTLED && wrote && got + LOST <= want.min(had) {
            note!(
                "after a fix in {} the field holds {got} chars, expected {want}; undone",
                self.in_front()
            );
            self.host.undo();
            self.expecting = None;
        } else if at.elapsed() >= Duration::from_secs(1) {
            note!(
                "after a fix in {} the field holds {got} chars, expected {want}",
                self.in_front()
            );
            self.expecting = None;
        }
    }

    /// Notes accepted fixes and takes them off the page at once, without
    /// waiting for the app to report the new text.
    fn remember(&mut self, fixes: &[Fix]) {
        let now = Instant::now();
        self.accepted.retain(|(_, at)| at.elapsed() < ACCEPTED_FOR);
        for fix in fixes {
            self.accepted.push((dismissal(&self.text, fix), now));
        }
        self.refilter();
    }

    /// Whether `fix` was accepted in the last `ACCEPTED_FOR`.
    pub(super) fn was_accepted(&self, fix: &Fix) -> bool {
        let key = dismissal(&self.text, fix);
        self.accepted
            .iter()
            .any(|(was, at)| *was == key && at.elapsed() < ACCEPTED_FOR)
    }

    /// A fix not applied, and why. Logged: losing the user's text is the one
    /// thing a correction must never do, so when in doubt nothing is written.
    fn refused(&self, why: Refusal) {
        note!("fix not applied in {}: {why}", self.in_front());
    }

    /// The app in front, for the log: a fix that goes wrong in one app and not
    /// another is the first thing to know.
    fn in_front(&self) -> String {
        self.host.front().map_or_else(|| "?".into(), |(id, _)| id)
    }
}
