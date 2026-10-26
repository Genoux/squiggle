//! The run loop and everything about suggestions that is not a platform:
//! watching the focused field, checking it when typing pauses, deciding what
//! each card says, and applying what the user accepts. A platform supplies a
//! [`Host`] (what is focused, where the pointer is) and an [`Overlay`] (what
//! is drawn), and `Engine` does the rest.

pub mod card;
pub mod dictionary;
mod engine;
mod geometry;
pub mod menu;
pub mod theme;
pub mod write;

use gramly_core::{Config, Kind};
use std::time::Instant;

pub use card::{Action, Bit, Card};
pub use engine::{DIALECTS, Engine, Status, TICK};
pub use geometry::{Point, Rect, in_corridor, place_card};
use write::Paste;

/// What a direct write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    /// The field now reads as expected.
    Done,
    /// The field is exactly as it was: the app ignored the write, and the
    /// paste may try.
    Ignored,
    /// The selection could not be confirmed, or the field changed into
    /// something else. Nothing more may be written: a paste now could land
    /// on the wrong text.
    Unsafe,
}

/// A text field in another app. Offsets are `char`s, as Harper reports them;
/// each platform converts to whatever its accessibility API counts.
pub trait Field: PartialEq {
    fn value(&self) -> Option<String>;
    /// The part of the field actually on screen.
    fn visible_frame(&self) -> Option<Rect>;
    /// Where a range is drawn. `None` and that fix is simply not drawn.
    fn bounds(&self, start: usize, len: usize) -> Option<Rect>;
    /// Selects a range of the value last read, and confirms the app
    /// selected exactly that text.
    fn select(&self, start: usize, len: usize) -> bool;
    /// Asked once more right before the paste keystroke: the selection made
    /// by `select` is still the selection. Where an app cannot be asked
    /// again cheaply this is true.
    fn still_selected(&self) -> bool {
        true
    }
    /// Puts the caret back where it was before `select`, when the fix is not
    /// going to be written: a refused fix leaves the field as it found it.
    fn deselect(&self) {}
    /// Replaces a range the way typing would. `before` is the field's text
    /// now, `expected` its text after: some apps accept the write and do
    /// nothing, so the result is read back rather than trusted.
    fn replace(&self, start: usize, len: usize, text: &str, before: &str, expected: &str) -> Write;
}

/// What has keyboard focus, as far as underlining goes.
pub enum Focus<F> {
    /// The field already being read.
    Same,
    /// Another editable text field.
    Field(F),
    /// An answer that it is not one: another control, a terminal, a password
    /// field, Gramly's own window, another app in front. The field is given
    /// up at once.
    Elsewhere,
    /// No answer: the app is busy or has just come forward. Not proof that
    /// focus left; see `engine::GRACE`.
    Unknown,
}

/// The operating system, as the engine asks about it.
pub trait Host {
    type Field: Field;
    /// Read every tick, so a setting applies on the next pause.
    fn config(&self) -> Config;
    fn trusted(&self) -> bool;
    /// The app in front as (id, name); `None` while Gramly itself is.
    fn front(&self) -> Option<(String, String)>;
    /// A terminal, skipped whatever the menu says.
    fn terminal(&self, id: &str) -> bool;
    /// What has keyboard focus, `current` being the field read last tick: a
    /// host can recognise it without asking the app anything about it. Takes
    /// `&mut` because a host may ask each app once to expose its text, and
    /// remembers the app in front: another one coming forward is an answer,
    /// not a miss, or the underlines sat over the next window for GRACE ticks.
    fn focus(&mut self, current: Option<&Self::Field>) -> Focus<Self::Field>;
    fn pointer(&self) -> Point;
    /// The clipboard half of the paste fallback.
    fn clipboard(&self) -> &dyn Paste;
    /// Undo in the focused app: takes back a fix that went wrong.
    fn undo(&self);
    /// The wall-clock time `at` will be, as the user's locale writes a time.
    fn clock(&self, at: Instant) -> String;
}

/// One underline.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub rect: Rect,
    pub kind: Kind,
    pub highlighted: bool,
    /// Just accepted: a green glow and no underline.
    pub flash: bool,
}

/// What is drawn on top of other apps. Which card is open is the engine's to
/// know; the overlay only draws what it is given.
pub trait Overlay {
    /// Underlines inside `area`; redrawn only when they changed.
    fn marks(&mut self, area: Rect, marks: Vec<Mark>);
    fn hide_marks(&mut self);
    fn show_card(&mut self, index: usize, card: &Card, word: Rect);
    fn hide_card(&mut self);
    /// The card's frame while it is on screen.
    fn card_frame(&self) -> Option<Rect>;
    /// The card's rows as (action, frame on screen), for `GRAMLY_STATE`.
    fn card_rows(&self) -> Vec<(Action, Rect)>;
    /// Lights the row under the pointer.
    fn hover(&mut self, pointer: Point);
}

/// Changes the saved settings. A failed save is logged: the menu has no
/// place to show it.
pub fn update_config(change: impl FnOnce(&mut Config)) {
    if let Err(err) = Config::update(change) {
        note!("saving settings: {err:#}");
    }
}

/// Lines of the log "Copy recent log" takes: a few minutes of use.
pub const LOG_LINES: usize = 200;

/// The end of the log, for a report: what went wrong is usually in the last
/// few minutes, and finding them in the file is work.
pub fn recent_log(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    lines[lines.len().saturating_sub(LOG_LINES)..].join("\n")
}

pub const BAD_KEY: &str = "That does not look like an OpenRouter key";

/// What a key field's text means: no key, a key, or a refusal.
pub fn parse_key(input: &str) -> Result<Option<String>, &'static str> {
    let key = input.trim();
    match key {
        "" => Ok(None),
        _ if gramly_core::router::valid_key(key) => Ok(Some(key.to_string())),
        _ => Err(BAD_KEY),
    }
}

/// The settings lines `gramly doctor` opens with.
pub fn config_lines(config: &Config) -> String {
    format!(
        "checking         {}\nai suggestions   {}\n",
        if config.enabled { "on" } else { "paused" },
        match (config.ai, config.openrouter_key.is_some()) {
            (true, true) => "on (key set)",
            (true, false) => "off - no OpenRouter key",
            (false, _) => "off",
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_tail_keeps_the_last_lines() {
        let log: String = (1..=LOG_LINES + 5).map(|n| format!("line {n}\n")).collect();
        let tail = recent_log(&log);
        assert_eq!(tail.lines().count(), LOG_LINES);
        assert!(tail.starts_with("line 6\n"));
        assert!(tail.ends_with(&format!("line {}", LOG_LINES + 5)));
    }

    #[test]
    fn an_empty_key_clears_the_key() {
        assert_eq!(parse_key("  \n"), Ok(None));
    }

    #[test]
    fn a_key_is_trimmed() {
        assert_eq!(
            parse_key(" sk-or-v1-abcdefghijklmnop \n"),
            Ok(Some("sk-or-v1-abcdefghijklmnop".to_string()))
        );
    }

    #[test]
    fn something_else_is_refused() {
        assert_eq!(parse_key("not a key!"), Err(BAD_KEY));
    }

    #[test]
    fn a_short_log_is_kept_whole() {
        assert_eq!(recent_log("a\nb\n"), "a\nb");
        assert_eq!(recent_log(""), "");
    }
}
