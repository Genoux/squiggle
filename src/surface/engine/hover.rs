//! The open card: which fix it shows, where the pointer is, and when it
//! closes.

use super::{Engine, HOVER, LEAVE, ORPHAN, trace};
use crate::surface::card::card;
use crate::surface::{Host, Overlay, Point, Rect, in_corridor};
use squiggle_core::{Fix, Kind, sentence};
use std::time::Instant;

impl<H: Host, O: Overlay> Engine<H, O> {
    /// Nothing left to underline - but a card still under the pointer stays
    /// until the pointer leaves it.
    pub(super) fn hold_orphan(&mut self) {
        if self.card_for != Some(ORPHAN) {
            self.hide_card();
            return;
        }
        let pointer = self.host.pointer();
        self.overlay.hover(pointer);
        self.linger(pointer);
    }

    pub(super) fn follow_pointer(&mut self) {
        if let (Some(word), Some(index)) = (self.card_stale.take(), self.card_for)
            && index != ORPHAN
        {
            self.hide_card();
            self.open_card(index, word);
        }
        let pointer = self.host.pointer();
        self.overlay.hover(pointer);
        if self.card_for.is_none() {
            self.set_card_fix(None);
            self.card_word = None;
        }
        if self.in_card_corridor(pointer) {
            // On the card, its word, or the way between: nothing closes and
            // nothing under the card can steal the hover.
            self.left_at = None;
            self.hovering = None;
        } else {
            self.hover_word(pointer);
        }
    }

    fn hover_word(&mut self, pointer: Point) {
        let over_word = self
            .words
            .iter()
            .find(|(_, rect)| rect.grow(2.0).contains(pointer))
            .copied();
        let Some((index, word)) = over_word else {
            return self.linger(pointer);
        };
        self.left_at = None;
        let since = match self.hovering {
            Some((current, since)) if current == index => since,
            _ => {
                let now = Instant::now();
                self.hovering = Some((index, now));
                now
            }
        };
        if since.elapsed() >= HOVER {
            self.open_card(index, word);
        }
    }

    /// Every fix in the same sentence as `index`, in order. One means the word
    /// card; more means the sentence card. An improvement is always alone: it
    /// is never part of "Correct the sentence".
    pub(super) fn group(&self, index: usize) -> Vec<(usize, Fix)> {
        let Some(fix) = self.fixes.get(index) else {
            return Vec::new();
        };
        if fix.kind == Kind::Clarity {
            return vec![(index, fix.clone())];
        }
        let (start, end) = sentence(&self.text, fix.start);
        self.fixes
            .iter()
            .enumerate()
            .filter(|(_, f)| f.kind != Kind::Clarity)
            // The hovered fix is always in its own group. One ending past its
            // sentence (a fix over "?" and what follows it) was left out, and
            // its card, given a group without it, opened nothing.
            .filter(|(i, f)| *i == index || f.start >= start && f.end <= end.max(start))
            .map(|(i, f)| (i, f.clone()))
            .collect()
    }

    /// The open card follows its fix through the rebuild: same words and same
    /// replacement, it stays exactly as it is; same words with a better
    /// replacement (the model's, after Harper's), it changes in place; gone,
    /// it closes. Closing it unconditionally is what made a card shown for
    /// "ok" vanish a second later when the model's answer arrived.
    pub(super) fn follow_fix(&mut self) {
        let (Some(shown), Some(word)) = (self.card_fix.clone(), self.card_word) else {
            self.hide_card();
            return;
        };
        match self
            .fixes
            .iter()
            .position(|f| f.start == shown.start && f.end == shown.end)
        {
            Some(index) if self.fixes[index] == shown => self.card_for = Some(index),
            Some(index) => {
                // Redrawn on the next tick, 50ms on; the content changes, the
                // card does not blink away.
                self.card_for = Some(index);
                self.set_card_fix(Some(self.fixes[index].clone()));
                self.card_stale = Some(word);
            }
            None if self.overlay.card_frame().is_some() => {
                note!("card's fix withdrawn; card kept until the pointer leaves");
                self.card_for = Some(ORPHAN);
            }
            None => self.hide_card(),
        }
    }

    /// The fix an action is about: the one the card is showing, not the index
    /// baked into the button when the card was drawn - the list can have been
    /// rebuilt since.
    pub(super) fn showing(&self, tagged: usize) -> usize {
        self.card_for.unwrap_or(tagged)
    }

    /// The fix the card shows, kept with the card itself so it can still be
    /// accepted after the list it came from was rebuilt.
    pub(super) fn shown_fix(&self, tagged: usize) -> Option<Fix> {
        self.card_fix
            .clone()
            .or_else(|| self.fixes.get(tagged).cloned())
    }

    fn set_card_fix(&mut self, fix: Option<Fix>) {
        self.card_words = fix.as_ref().map(|fix| fix.original(&self.text));
        self.card_fix = fix;
    }

    pub(super) fn hide_card(&mut self) {
        self.overlay.hide_card();
        self.card_for = None;
    }

    fn open_card(&mut self, index: usize, word: Rect) {
        let members = self.group(index);
        self.set_card_fix(self.fixes.get(index).cloned());
        self.card_word = Some(word);
        if self.card_for == Some(index) && self.overlay.card_frame().is_some() {
            return;
        }
        let Some(content) = card(index, &members, &self.text) else {
            return;
        };
        self.overlay.show_card(index, &content, word);
        self.card_for = Some(index);
    }

    fn in_card_corridor(&self, pointer: Point) -> bool {
        let (Some(card), Some(word)) = (self.overlay.card_frame(), self.card_word) else {
            return false;
        };
        in_corridor(card, word, pointer)
            || self
                .overlay
                .menu_frame()
                .is_some_and(|menu| menu.contains(pointer))
    }

    /// The pointer is off every word: the card closes once it has been out of the
    /// corridor for `LEAVE`.
    fn linger(&mut self, pointer: Point) {
        self.hovering = None;
        if self.card_for.is_none() {
            return;
        }
        if self.in_card_corridor(pointer) {
            self.left_at = None;
            return;
        }
        let left = *self.left_at.get_or_insert_with(Instant::now);
        if left.elapsed() >= LEAVE {
            trace("pointer left the card; card closed");
            self.hide_card();
            self.left_at = None;
        }
    }
}
