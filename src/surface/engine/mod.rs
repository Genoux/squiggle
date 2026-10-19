//! The run loop: watch the focused field, check it when typing pauses, draw
//! what was found, and apply what the user accepts.
//!
//! Polled rather than driven by AXObserver notifications. Notifications are
//! per process and per element, arrive for some apps and not others (several
//! Electron builds never send `AXValueChanged`), and must be re-registered
//! every time focus moves; a 50ms poll (`TICK`) of one attribute costs well
//! under a millisecond and behaves the same in every app.

mod accept;
mod export;
mod hover;
#[cfg(test)]
mod tests;

use super::card::Action;
use super::{Field, Focus, Host, Mark, Overlay, Rect, update_config};
use gramly_core::diff::{Piece, pieces};
use gramly_core::worker::{Outcome, Worker};
use gramly_core::{Config, Fix, Kind, config, merge, rebase, sentence};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// 50ms: hover is a poll of the pointer, so this is the card's worst-case
/// latency. At 100ms the card read as sluggish next to Grammarly's.
pub const TICK: Duration = Duration::from_millis(50);
/// Typing must pause this long before a check is asked for. Underlines no
/// longer clear while typing (see `rebase`), so this only decides how soon a
/// *new* mistake is marked; 450ms felt late.
const PAUSE: Duration = Duration::from_millis(250);
/// Hovering a word this long opens its card - just enough that sweeping the
/// pointer across a paragraph does not flash cards open.
const HOVER: Duration = Duration::from_millis(40);
/// Leaving the card's corridor (see `in_corridor`) for this long closes it.
/// Generous on purpose: a card that vanishes on a wobble is the one thing
/// that makes the whole app feel broken, and one that lingers a beat costs
/// nothing.
const LEAVE: Duration = Duration::from_millis(320);
/// Unanswered questions in a row before the field is given up: 8 ticks,
/// 400ms. Fewer and a busy app closed the card mid-read.
const GRACE: u32 = 8;
/// Past this without an answer from the model, Harper's underlines are
/// shown after all.
const MODEL_LATE: Duration = Duration::from_secs(4);
/// How long an accepted fix stays accepted: not offered, not written again.
const ACCEPTED_FOR: Duration = Duration::from_secs(60);
/// How long an accepted word glows.
pub const FLASH: Duration = Duration::from_millis(380);
/// "Pause for an hour". Held in memory: a restart is a fresh start.
const PAUSE_FOR: Duration = Duration::from_secs(60 * 60);
/// The Spelling submenu, in the config's words and the menu's.
pub const DIALECTS: &[(&str, &str)] = &[
    ("american", "American"),
    ("british", "British"),
    ("canadian", "Canadian"),
    ("australian", "Australian"),
    ("indian", "Indian"),
];
/// Dismissals kept; past this the oldest are not worth tracking, and all
/// are forgotten at once rather than kept in order.
const DISMISSALS: usize = 2000;
/// `card_for` for a card whose fix was withdrawn while the pointer was on it.
/// It stays until the pointer leaves - a card that vanishes from under the
/// pointer reads as a bug, whatever the reason - and indexes nothing.
const ORPHAN: usize = usize::MAX;

const LOCAL_ONLY: &str = if cfg!(target_os = "macos") {
    "Checking on this Mac only"
} else {
    "Checking on this computer only"
};

/// A dismissed suggestion: the few words before it on its line, what it
/// would change, and to what. The words before keep a dismissal to that
/// place - "ok" -> "OK" dismissed once is not dismissed everywhere - and,
/// unlike the whole sentence, they do not change as the sentence goes on
/// being typed. Editing just before it brings the suggestion back.
type Dismissal = (String, String, String);

/// What the menu shows. Drawn again only when it differs from the last
/// one drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub line: String,
    pub trusted: bool,
    pub enabled: bool,
    pub ai: bool,
    pub paused: bool,
    pub front: Option<(String, String)>,
    pub off_here: bool,
    pub terminal: bool,
    pub dialect: String,
}

/// The per-app menu item: shown only while an app is in front.
#[derive(Debug, PartialEq)]
pub struct AppItem {
    pub title: String,
    pub enabled: bool,
}

impl Status {
    pub fn pause_title(&self) -> &'static str {
        if self.paused {
            "Resume checking"
        } else {
            "Pause for an hour"
        }
    }

    pub fn app_item(&self) -> Option<AppItem> {
        let (_, name) = self.front.as_ref()?;
        let (title, enabled) = if self.terminal {
            (format!("Off in {name}: terminals are skipped"), false)
        } else if self.off_here {
            (format!("Turn on in {name}"), true)
        } else {
            (format!("Turn off in {name}"), true)
        };
        Some(AppItem { title, enabled })
    }
}

pub struct Engine<H: Host, O: Overlay> {
    host: H,
    overlay: O,
    worker: Worker,
    /// What `gate` read this tick, so a setting applies on the next pause
    /// and the menu is drawn from the same read.
    config: Config,
    field: Option<H::Field>,
    text: String,
    changed_at: Instant,
    generation: u64,
    submitted: bool,
    local: Vec<Fix>,
    ai: Vec<Fix>,
    /// Clearer wording from `improve`, drawn only beside a settled
    /// correction answer; see `refilter`.
    improve: Vec<Fix>,
    fixes: Vec<Fix>,
    /// What was dismissed, as [`dismissal`] keys it. Kept across fields and
    /// apps until Gramly quits: cleared whenever focus moved, a suggestion
    /// dismissed in Slack was back after a look at another window.
    dismissed: HashSet<Dismissal>,
    /// Each drawn fix's word rectangle, by index into `fixes`.
    words: Vec<(usize, Rect)>,
    hovering: Option<(usize, Instant)>,
    left_at: Option<Instant>,
    /// The fix the card is showing.
    card_for: Option<usize>,
    /// The open card's fix and the word it hangs from. Kept so the card
    /// survives the fix list being rebuilt under it (the model's answer
    /// arriving), and so the corridor knows where the word is.
    card_fix: Option<Fix>,
    /// What `card_fix` covered when the card was shown: an accept from a card
    /// that outlived its fix checks it still does.
    card_words: Option<String>,
    card_word: Option<Rect>,
    /// Words just accepted, glowing until the instant passes.
    flashing: Option<(Vec<Rect>, Instant)>,
    /// The open card's fix changed under it; redraw it at this word.
    card_stale: Option<Rect>,
    /// The field's text before the last accepted fix, what it should read
    /// after, and when it was accepted; see `audit`.
    expecting: Option<(String, String, Instant)>,
    /// Fixes accepted in the last `ACCEPTED_FOR`, by the same key as a
    /// dismissal, and when. None of them is offered or written again.
    accepted: Vec<(Dismissal, Instant)>,
    /// Unanswered accessibility questions in a row; see `GRACE`.
    misses: u32,
    /// When the current text was sent to be checked.
    submitted_at: Instant,
    /// Where each fix was last drawn, kept while the text is unchanged so a
    /// word the app fails to place for a tick keeps its underline.
    placed: HashMap<(usize, usize), Rect>,
    /// Harper has been shown for the current text because the model was late.
    fell_back: bool,
    ai_error: Option<String>,
    /// The model has answered for the text as it is now. Until then Harper's
    /// underlines stand in; once it has, its answer is the whole answer.
    ai_current: bool,
    trusted: bool,
    /// "Pause for an hour" ends here; the clock time is written once, when
    /// the pause starts, since the menu asks for the status every tick.
    paused_until: Option<(Instant, String)>,
    /// The app in front (bundle id, name); see [`Host::front`].
    front: Option<(String, String)>,
    /// The last snapshot written for `GRAMLY_STATE`.
    exported: Option<String>,
}

impl<H: Host, O: Overlay> Engine<H, O> {
    pub fn new(host: H, overlay: O) -> Self {
        let trusted = host.trusted();
        Self {
            config: host.config(),
            host,
            overlay,
            worker: Worker::spawn(),
            field: None,
            text: String::new(),
            changed_at: Instant::now(),
            generation: 0,
            submitted: false,
            local: Vec::new(),
            ai: Vec::new(),
            improve: Vec::new(),
            fixes: Vec::new(),
            dismissed: HashSet::new(),
            words: Vec::new(),
            hovering: None,
            left_at: None,
            card_for: None,
            card_fix: None,
            card_words: None,
            card_word: None,
            flashing: None,
            card_stale: None,
            expecting: None,
            accepted: Vec::new(),
            misses: 0,
            submitted_at: Instant::now(),
            placed: HashMap::new(),
            fell_back: false,
            ai_error: None,
            ai_current: false,
            trusted,
            paused_until: None,
            front: None,
            exported: None,
        }
    }

    pub fn tick(&mut self) {
        self.poll();
        self.export();
    }

    /// AI suggestions are switched on and have a key.
    fn ai_on(&self) -> bool {
        self.config.ai && self.config.openrouter_key.is_some()
    }

    pub fn status(&self) -> Status {
        let config = &self.config;
        let off_here = self
            .front
            .as_ref()
            .filter(|(id, _)| config.off_in.contains(id));
        let line = if !self.trusted {
            "Needs Accessibility access".to_string()
        } else if !config.enabled {
            "Paused".to_string()
        } else if let Some((_, clock)) = &self.paused_until {
            format!("Paused until {clock}")
        } else if let Some((_, name)) = off_here {
            format!("Off in {name}")
        } else if !self.ai_on() {
            LOCAL_ONLY.to_string()
        } else if let Some(error) = &self.ai_error {
            format!("Model unavailable: {}", truncate(error, 48))
        } else {
            "Checking with Gemini 3.5 Flash-Lite".to_string()
        };
        Status {
            line,
            trusted: self.trusted,
            enabled: config.enabled,
            ai: self.ai_on(),
            paused: self.paused_until.is_some(),
            front: self.front.clone(),
            off_here: off_here.is_some(),
            terminal: self
                .front
                .as_ref()
                .is_some_and(|(id, _)| self.host.terminal(id)),
            dialect: config.dialect.clone(),
        }
    }

    /// What clicking a row of the card does. `tagged` is the index the row
    /// was drawn with, which only matters when the card shows nothing.
    pub fn act(&mut self, action: Action, tagged: usize) {
        match action {
            Action::Accept => self.accept_shown(tagged),
            Action::Dismiss => self.dismiss_shown(tagged),
            Action::Learn => self.learn_shown(tagged),
            Action::AcceptSentence => self.accept_group(tagged),
            Action::DismissSentence => self.dismiss_group(tagged),
        }
    }

    fn accept_shown(&mut self, tagged: usize) {
        let Some(fix) = self.shown_fix(tagged) else {
            return;
        };
        if let Some(word) = self.card_word {
            self.flashing = Some((vec![word], Instant::now()));
        }
        self.accept_fix(&fix);
    }

    fn dismiss_shown(&mut self, tagged: usize) {
        if let Some(fix) = self.shown_fix(tagged) {
            self.dismissed.insert(dismissal(&self.text, &fix));
        }
        self.hide_card();
        self.refilter();
    }

    fn learn_shown(&mut self, tagged: usize) {
        if let Some(fix) = self.shown_fix(tagged) {
            let word = fix.original(&self.text);
            if let Err(err) = config::add_to_dictionary(&word) {
                note!("adding {word:?} to the dictionary: {err:#}");
            }
            self.dismissed.insert(dismissal(&self.text, &fix));
        }
        self.hide_card();
        self.refilter();
        // Re-check so the other places that word was flagged clear too.
        self.recheck();
    }

    fn accept_group(&mut self, tagged: usize) {
        let index = self.showing(tagged);
        let indices: Vec<usize> = self.group(index).into_iter().map(|(i, _)| i).collect();
        if indices.is_empty() {
            // The card outlived its fixes (see `ORPHAN`): take the one shown.
            if let Some(fix) = self.shown_fix(tagged) {
                self.accept_fix(&fix);
            }
            return;
        }
        self.flash(&indices);
        self.accept_fixes(&indices);
    }

    fn dismiss_group(&mut self, tagged: usize) {
        let index = self.showing(tagged);
        for (_, fix) in self.group(index) {
            self.dismissed.insert(dismissal(&self.text, &fix));
        }
        self.hide_card();
        self.refilter();
    }

    /// Asks for the text to be checked again now, as if typing had just paused:
    /// after the dictionary changes, the underlines should follow at once.
    pub fn recheck(&mut self) {
        self.submitted = false;
        self.changed_at = Instant::now() - PAUSE;
    }

    pub fn clear_ai_error(&mut self) {
        self.ai_error = None;
    }

    pub fn toggle_pause(&mut self) {
        self.paused_until = match self.paused_until {
            Some(_) => None,
            None => {
                let until = Instant::now() + PAUSE_FOR;
                Some((until, self.host.clock(until)))
            }
        };
        self.reset();
    }

    pub fn toggle_app(&mut self) {
        let Some((id, _)) = self.front.clone() else {
            return;
        };
        update_config(|c| {
            if c.off_in.contains(&id) {
                c.off_in.retain(|other| *other != id);
            } else {
                c.off_in.push(id.clone());
            }
        });
        self.reset();
    }

    pub fn reset(&mut self) {
        self.field = None;
        self.text.clear();
        self.placed.clear();
        self.local.clear();
        self.ai.clear();
        self.improve.clear();
        self.fixes.clear();
        self.words.clear();
        self.hovering = None;
        self.left_at = None;
        self.submitted = false;
        self.ai_current = false;
        // Another field's text is no measure of the fix: ⌘Z there would undo
        // something the user did.
        self.expecting = None;
        self.generation += 1;
        self.overlay.hide_marks();
        self.hide_card();
    }

    /// Counts an unanswered question; after `GRACE` in a row the field is
    /// dropped and everything hidden.
    fn missed(&mut self, what: &str) {
        self.misses += 1;
        if self.misses < GRACE {
            return;
        }
        if self.field.is_some() {
            trace(&format!(
                "{what} unanswered for {GRACE} ticks; field dropped"
            ));
        }
        self.reset();
    }

    fn poll(&mut self) {
        if self.gate()
            && self.follow_focus()
            && self.read_text()
            && self.take_answers()
            && let Some(field_rect) = self.place_words()
        {
            self.follow_pointer();
            self.draw(field_rect);
        }
    }

    /// Reads the settings and the app in front; false when nothing should be
    /// read or drawn, with everything hidden.
    fn gate(&mut self) -> bool {
        self.config = self.host.config();
        self.trusted = self.host.trusted();
        let front = self.host.front();
        if front.is_some() && front != self.front {
            self.front = front;
        }
        if self
            .paused_until
            .as_ref()
            .is_some_and(|(until, _)| Instant::now() >= *until)
        {
            self.paused_until = None;
        }
        let off_here = self
            .front
            .as_ref()
            .is_some_and(|(id, _)| self.config.off_in.contains(id));
        let open = self.config.enabled && self.trusted && self.paused_until.is_none() && !off_here;
        if !open {
            self.reset();
        }
        open
    }

    /// Whether a field is focused. A question to another app can go
    /// unanswered for a tick - Chromium misses its 150ms deadline while it is
    /// busy - and that is not focus leaving. Only a run of misses is. An
    /// answer that focus is on something else is, at once.
    fn follow_focus(&mut self) -> bool {
        match self.host.focus(self.field.as_ref()) {
            Focus::Same => self.misses = 0,
            Focus::Field(field) => {
                self.reset();
                self.field = Some(field);
                self.misses = 0;
            }
            Focus::Elsewhere => {
                if self.field.is_some() {
                    trace("focus moved off the field");
                    self.reset();
                }
                return false;
            }
            Focus::Unknown => {
                if self.field.is_some() {
                    self.missed("focus");
                }
                return false;
            }
        }
        true
    }

    fn read_text(&mut self) -> bool {
        let Some(text) = self.field.as_ref().and_then(|field| field.value()) else {
            self.missed("text");
            return false;
        };
        if text != self.text {
            self.follow_edit(text);
        }
        self.audit();
        if !self.submitted && self.changed_at.elapsed() >= PAUSE {
            self.submitted = true;
            self.submitted_at = Instant::now();
            self.worker.submit(self.generation, self.text.clone());
        }
        true
    }

    /// The text changed. The underlines the edit did not touch are carried
    /// across rather than cleared, so typing and accepting never make the
    /// rest flicker.
    fn follow_edit(&mut self, text: String) {
        self.local = rebase(&self.text, &text, &self.local);
        self.ai = rebase(&self.text, &text, &self.ai);
        self.improve = rebase(&self.text, &text, &self.improve);
        self.fixes = rebase(&self.text, &text, &self.fixes);
        trace(&format!(
            "text changed: {} -> {} chars",
            self.text.chars().count(),
            text.chars().count()
        ));
        self.text = text;
        self.placed.clear();
        self.fell_back = false;
        self.ai_current = false;
        self.changed_at = Instant::now();
        self.submitted = false;
        self.generation += 1;
        if self.card_for.is_some() {
            trace("text changed; card closed");
        }
        self.hide_card();
    }

    /// Takes what the checker answered; false when there is nothing to draw.
    fn take_answers(&mut self) -> bool {
        let mut changed = self.drain_outcomes();
        // The model is late or failing: show what Harper found, once.
        let late = self.submitted && self.submitted_at.elapsed() >= MODEL_LATE;
        if !self.ai_current && !self.fell_back && (late || self.ai_error.is_some()) {
            self.fell_back = true;
            changed = true;
        }
        if changed {
            self.refilter();
        }
        if self.fixes.is_empty() {
            self.overlay.hide_marks();
            self.hold_orphan();
            return false;
        }
        true
    }

    /// Whether an answer changed what is known about the current text.
    fn drain_outcomes(&mut self) -> bool {
        let mut changed = false;
        while let Some(outcome) = self.worker.try_recv() {
            match outcome {
                Outcome::Local { generation, fixes } if generation == self.generation => {
                    self.local = fixes;
                    changed = true;
                }
                Outcome::Ai { generation, fixes } if generation == self.generation => {
                    let took = self.submitted_at.elapsed();
                    if took > Duration::from_secs(2) {
                        note!(
                            "the model answered {:.1}s after the pause",
                            took.as_secs_f64()
                        );
                    }
                    self.ai = fixes;
                    self.ai_current = true;
                    self.ai_error = None;
                    changed = true;
                }
                Outcome::Improve { generation, fixes } if generation == self.generation => {
                    self.improve = fixes;
                    changed = true;
                }
                Outcome::ImproveFailed { reason, .. } => {
                    note!("improvement check failed: {reason}");
                }
                Outcome::AiFailed { reason, .. } if self.ai_error.as_deref() != Some(&reason) => {
                    note!("model check failed: {reason}");
                    self.ai_error = Some(reason);
                }
                _ => {}
            }
        }
        changed
    }

    /// Every tick, not only when the text changes: the field scrolls and its
    /// window moves without it. Returns the field's visible frame.
    fn place_words(&mut self) -> Option<Rect> {
        let Some(field_rect) = self.field.as_ref().and_then(|f| f.visible_frame()) else {
            self.missed("frame");
            return None;
        };
        self.misses = 0;
        let field = self.field.as_ref()?;
        let placed = &mut self.placed;
        self.words = self
            .fixes
            .iter()
            .enumerate()
            .filter_map(|(index, fix)| {
                locate(field, &self.text, &anchor(&self.text, fix), placed)
                    .filter(|word| scrolled_into(*word, field_rect))
                    .map(|word| (index, word))
            })
            .collect();
        Some(field_rect)
    }

    fn draw(&mut self, field_rect: Rect) {
        // Everything the open card would change is lit, so a sentence card
        // shows which words "Correct the sentence" touches.
        let highlighted: Vec<usize> = self
            .card_for
            .map(|i| self.group(i).into_iter().map(|(i, _)| i).collect())
            .unwrap_or_default();
        let mut marks: Vec<Mark> = self
            .words
            .iter()
            .map(|(index, rect)| Mark {
                rect: *rect,
                kind: self.fixes[*index].kind,
                highlighted: highlighted.contains(index),
                flash: false,
            })
            .collect();
        if let Some((rects, since)) = &self.flashing {
            if since.elapsed() < FLASH {
                marks.extend(rects.iter().map(|rect| Mark {
                    rect: *rect,
                    kind: Kind::Correctness,
                    highlighted: true,
                    flash: true,
                }));
            } else {
                self.flashing = None;
            }
        }
        self.overlay.marks(field_rect.grow(4.0), marks);
    }

    fn refilter(&mut self) {
        if self.dismissed.len() > DISMISSALS {
            self.dismissed.clear();
        }
        // An accepted fix is not offered again. The app can take a moment to
        // report the text the fix wrote - Slack did - and until it does, the
        // old text still holds the fix: offered again and accepted again,
        // "new" got two commas.
        let text = &self.text;
        let fresh = |fix: &Fix| {
            let key = dismissal(text, fix);
            !self.dismissed.contains(&key) && !self.accepted.iter().any(|(was, _)| *was == key)
        };
        let mut fixes: Vec<Fix> = self
            .shown_fixes()
            .into_iter()
            .filter(|fix| fresh(fix))
            .collect();
        // Improvements are proposals on correct text. One is shown only once
        // the model has answered for this text, and only in a sentence with
        // no correction left: rewriting words that are about to be corrected
        // would have the two cards fight over them. Accepting the corrections
        // changes the text, and the next answer proposes the rewrite of what
        // is left.
        if self.ai_current {
            let corrected: Vec<(usize, usize)> =
                fixes.iter().map(|fix| sentence(text, fix.start)).collect();
            let improvements: Vec<Fix> = self
                .improve
                .iter()
                .filter(|fix| !corrected.contains(&sentence(text, fix.start)))
                .filter(|fix| fresh(fix))
                .cloned()
                .collect();
            fixes.extend(improvements);
            fixes.sort_by_key(|fix| fix.start);
        }
        self.fixes = fixes;
        if std::env::var_os("GRAMLY_TRACE").is_some() {
            let shown: Vec<String> = self
                .fixes
                .iter()
                .map(|f| format!("{:?}->{:?}", f.original(&self.text), f.replacement))
                .collect();
            trace(&format!(
                "fixes (local {}, model {}, model answered {}): {shown:?}",
                self.local.len(),
                self.ai.len(),
                self.ai_current
            ));
        }
        self.follow_fix();
    }

    /// The model read the sentence; Harper matched patterns. Harper is the
    /// instant first draw and the offline fallback, and on French slang it
    /// offered "Eat" for "ERT" and "ma's" for "mais" - so once the model has
    /// answered for this text, what it did not flag is not flagged.
    ///
    /// And with the model on, Harper is not drawn at all while waiting for it.
    /// Drawing Harper first and the model second meant every suggestion the
    /// model disagreed with appeared, then vanished - often while being
    /// hovered, leaving a card with no underline to come back to. Underlines
    /// that arrive a second later and stay beat underlines that arrive at
    /// once and change their mind. Harper still draws when the model is off,
    /// failing, or later than `MODEL_LATE`.
    ///
    /// Not even Harper's unknown words. Kept beside the model's, they offered
    /// "tmrw" -> "thaw" and "oldad" -> "olds": the model left the shorthand
    /// alone on purpose and fixed the typo better. An app that is wrong in
    /// plain sight is not trusted on anything else.
    fn shown_fixes(&self) -> Vec<Fix> {
        let model_coming =
            self.ai_on() && self.ai_error.is_none() && self.submitted_at.elapsed() < MODEL_LATE;
        if self.ai_current || model_coming {
            self.ai.clone()
        } else {
            merge(self.local.clone(), self.ai.clone())
        }
    }
}

/// What an underline is drawn under. A correction is drawn under all it
/// changes; an improvement spans most of a sentence and may wrap, so it is
/// drawn under the first words it changes ("for now" in "Can you think for
/// now about conservation."), as Grammarly does.
fn anchor(text: &str, fix: &Fix) -> Fix {
    if fix.kind != Kind::Clarity {
        return fix.clone();
    }
    let mut at = fix.start;
    for piece in pieces(&fix.original(text), &fix.replacement) {
        match piece {
            Piece::Same(same) => at += same.chars().count(),
            Piece::Changed { was, .. } => {
                let lead = was.chars().take_while(|c| c.is_whitespace()).count();
                let mut part = fix.clone();
                part.start = at + lead;
                part.end = at + was.trim_end().chars().count().max(lead);
                return part;
            }
        }
    }
    fix.clone()
}

/// Where a fix is drawn: its own range, the character before an insertion,
/// or the last place it was drawn when the app fails to answer for a tick.
fn locate<F: Field>(
    field: &F,
    text: &str,
    fix: &Fix,
    placed: &mut HashMap<(usize, usize), Rect>,
) -> Option<Rect> {
    // An insertion has no width of its own; underline the character it
    // follows so there is something to hover.
    let (start, len) = match fix.end - fix.start {
        0 => (fix.start.saturating_sub(1), 1),
        len => (fix.start, len),
    };
    let key = (fix.start, fix.end);
    let spacing = fix.original(text).chars().all(char::is_whitespace);
    field
        .bounds(start, len)
        // A range that is only spaces may not be placed; a doubled space is
        // drawn just after the character before it.
        .or_else(|| {
            let before = field.bounds(start.checked_sub(1)?, 1)?;
            spacing.then_some(Rect {
                x: before.x + before.w,
                y: before.y,
                w: 8.0,
                h: before.h,
            })
        })
        .inspect(|bounds| {
            placed.insert(key, *bounds);
        })
        .or_else(|| placed.get(&key).copied())
}

/// Whether a word is within the field's rows, with a little slack: scrolled
/// out of the field's visible area it is not drawn.
fn scrolled_into(word: Rect, field: Rect) -> bool {
    word.y >= field.y - 2.0 && word.y + word.h <= field.y + field.h + 2.0
}

fn dismissal(text: &str, fix: &Fix) -> Dismissal {
    let before: String = text.chars().take(fix.start).collect();
    let line = before.rsplit('\n').next().unwrap_or_default();
    let words: Vec<&str> = line.split_whitespace().collect();
    let context = words[words.len().saturating_sub(3)..].join(" ");
    (context, fix.original(text), fix.replacement.clone())
}

/// Why the card or the underlines went away, with `GRAMLY_TRACE` set. A card
/// "hiding for no reason" is only fixable with the reason.
fn trace(why: &str) {
    if std::env::var_os("GRAMLY_TRACE").is_some() {
        note!("{why}");
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max - 1).collect();
    format!("{cut}…")
}
