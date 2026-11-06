//! Selecting and writing in a field: the calls that change what the user
//! sees, apart from reading it.

use super::Field;
use super::padding::{app_range, strip_padding};
use crate::linux::edit;
use crate::surface::Write;
use crate::surface::write::{classify, read_back, settle};
use atspi_proxies::accessible::AccessibleProxyBlocking;
use atspi_proxies::common::State;
use atspi_proxies::editable_text::EditableTextProxyBlocking;
use atspi_proxies::text::TextProxyBlocking;

impl Field {
    /// Selects `len` characters from `start` of the value last read, and
    /// whether the app confirmed exactly that range holding exactly that
    /// text. The caret is read first: a selection that is refused or never
    /// pasted over must be taken back.
    pub(super) fn select_range(&self, start: usize, len: usize) -> Option<bool> {
        let wanted = edit::span(&self.text.borrow(), start, len)?;
        let (from, to) = app_range(&self.padding.borrow(), start, len)?;
        let text = proxy!(self, TextProxyBlocking);
        self.caret.set(text.caret_offset().ok());
        self.selected.set(None);
        // Gecko keeps the caret as a range of its own beside the one
        // `AddSelection` adds, and reports only the added one; a paste then
        // cleared the selection and typed at the caret. With the caret at 0
        // in Zen, "teh" -> "the" read "theThe weather was nice and  sun".
        // Moved to the start first, both ranges begin where the fix does.
        // Waited for, because Chromium applies each request later, in its
        // renderer: a caret move that landed after the selection collapsed
        // it, and the paste read "theteh".
        text.set_caret_offset(from).ok()?;
        if !settle(|| text.caret_offset().ok() == Some(from)) {
            return Some(false);
        }
        if text.get_n_selections().ok()? > 0 {
            text.set_selection(0, from, to).ok()?;
        } else {
            text.add_selection(from, to).ok()?;
        }
        let selected = || {
            text.get_selection(0).ok() == Some((from, to))
                && stripped(&text, from, to).as_deref() == Some(wanted.as_str())
        };
        if !settle(selected) {
            return Some(false);
        }
        // Still selected a moment on: a request still on its way would
        // have changed it by now.
        std::thread::sleep(edit::STEADY);
        let held = selected();
        self.selected.set(held.then_some((from, to)));
        Some(held)
    }

    /// Whether the app still reports the selection `select_range` made, in a
    /// field that still has focus: a field keeps its selection when the user
    /// clicks into another one in the same window, and Ctrl+V would go there.
    pub(super) fn selection_holds(&self) -> bool {
        let held = || {
            let focused = proxy!(self, AccessibleProxyBlocking)
                .get_state()
                .ok()?
                .contains(State::Focused);
            let text = proxy!(self, TextProxyBlocking);
            Some(focused && text.get_selection(0).ok()? == self.selected.get()?)
        };
        held().unwrap_or(false)
    }

    /// Drops the selection and puts the caret where it was before
    /// `select_range` moved it.
    pub(super) fn put_caret_back(&self) -> Option<()> {
        let caret = self.caret.take()?;
        self.selected.set(None);
        let text = proxy!(self, TextProxyBlocking);
        if text.get_n_selections().ok()? > 0 {
            text.remove_selection(0).ok()?;
        }
        text.set_caret_offset(caret).ok()?;
        Some(())
    }

    /// Replaces the range through `EditableText` and reads the field back.
    /// A write that stopped halfway, the delete taken and the insert not, is
    /// put back before the outcome is judged: a field left short of the
    /// range is neither ignored nor done.
    pub(super) fn write_native(
        &self,
        start: usize,
        len: usize,
        text: &str,
        before: &str,
        expected: &str,
    ) -> Write {
        let Some((from, to)) = self.confirm(start, len, before) else {
            return Write::Unsafe;
        };
        // A call that errored may still have landed: the read-back decides.
        // It reads untimed: one slow read would start the field's cooldown,
        // every read after it would come back empty, and a half-finished
        // write could be neither recognised nor put back.
        let _ = self.type_over(from, to, text);
        let mut after = read_back(|| self.read_value(), expected);
        if edit::half_written(before, after.as_deref(), expected, start, len) {
            after = self.put_back(from, before, (start, len));
        }
        classify(before, after.as_deref(), expected)
    }

    /// The app's range for `len` characters from `start`, if it holds the
    /// characters `before` says it does.
    fn confirm(&self, start: usize, len: usize, before: &str) -> Option<(i32, i32)> {
        let wanted = edit::span(before, start, len)?;
        let (from, to) = app_range(&self.padding.borrow(), start, len)?;
        let text = proxy!(self, TextProxyBlocking);
        (stripped(&text, from, to)? == wanted).then_some((from, to))
    }

    fn type_over(&self, from: i32, to: i32, text: &str) -> Option<()> {
        if to > from {
            proxy!(self, EditableTextProxyBlocking)
                .delete_text(from, to)
                .ok()?;
        }
        self.insert(from, text)
    }

    fn insert(&self, at: i32, text: &str) -> Option<()> {
        let editable = proxy!(self, EditableTextProxyBlocking);
        // ATK counts `length` in bytes.
        editable
            .insert_text(at, text, i32::try_from(text.len()).ok()?)
            .ok()?;
        Some(())
    }

    /// Inserts back the text a half-finished write deleted, and reads the
    /// field once it is `before` again.
    fn put_back(&self, from: i32, before: &str, (start, len): (usize, usize)) -> Option<String> {
        let original = edit::span(before, start, len)?;
        let _ = self.insert(from, &original);
        read_back(|| self.read_value(), before)
    }
}

/// The text between two app offsets, as Squiggle counts it.
fn stripped(text: &TextProxyBlocking, from: i32, to: i32) -> Option<String> {
    Some(strip_padding(&text.get_text(from, to).ok()?).0)
}
