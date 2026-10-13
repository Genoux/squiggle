//! The run loop: watch the focused field, check it when typing pauses, draw
//! what was found, and apply what the user accepts.
//!
//! Polled rather than driven by AXObserver notifications. Notifications are
//! per process and per element, arrive for some apps and not others (several
//! Electron builds never send `AXValueChanged`), and must be re-registered
//! every time focus moves; a 100ms poll of one attribute costs well under a
//! millisecond and behaves the same in every app.

use crate::macos::ax::{self, Field};
use crate::macos::overlay::{self, Mark, Overlay};
use crate::macos::paste;
use gramly_core::worker::{Outcome, Worker};
use gramly_core::{Config, Fix, apply, config, merge, rebase, sentence};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationPolicy, NSEvent,
    NSImage, NSMenu, NSMenuItem, NSSecureTextField, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength, NSWorkspace,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSString, NSTimer, NSURL};
use std::cell::RefCell;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// 50ms: hover is a poll of the pointer, so this is the card's worst-case
/// latency. At 100ms the card read as sluggish next to Grammarly's.
const TICK: f64 = 0.05;
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
/// How long an accepted word glows green.
const FLASH: Duration = Duration::from_millis(380);
const VERSION: &str = env!("CARGO_PKG_VERSION");

struct State {
    worker: Worker,
    overlay: Overlay,
    field: Option<Field>,
    text: String,
    changed_at: Instant,
    generation: u64,
    submitted: bool,
    local: Vec<Fix>,
    ai: Vec<Fix>,
    fixes: Vec<Fix>,
    /// `(original, replacement)` pairs dismissed in this field. Cleared when
    /// focus moves, as Grammarly's are.
    dismissed: HashSet<(String, String)>,
    /// Each drawn fix's word rectangle, Cocoa coordinates, by index into `fixes`.
    words: Vec<(usize, CGRect)>,
    hovering: Option<(usize, Instant)>,
    left_at: Option<Instant>,
    /// The open card's fix and the word it hangs from. Kept so the card
    /// survives the fix list being rebuilt under it (the model's answer
    /// arriving), and so the corridor knows where the word is.
    card_fix: Option<Fix>,
    card_word: Option<CGRect>,
    /// Words just accepted, glowing green until the instant passes.
    flashing: Option<(Vec<CGRect>, Instant)>,
    /// The open card's fix changed under it; redraw it at this word.
    card_stale: Option<CGRect>,
    /// Apps already asked to expose their text (see `ax::wake`).
    woken: HashSet<i32>,
    ai_error: Option<String>,
    /// The model has answered for the text as it is now. Until then Harper's
    /// underlines stand in; once it has, its answer is the whole answer.
    ai_current: bool,
    trusted: bool,
    status: Option<Retained<NSStatusItem>>,
    menu_status: Option<Retained<NSMenuItem>>,
    menu_enabled: Option<Retained<NSMenuItem>>,
    menu_ai: Option<Retained<NSMenuItem>>,
    menu_access: Option<Retained<NSMenuItem>>,
}

pub struct Ivars {
    state: RefCell<Option<State>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GramlyController"]
    #[ivars = Ivars]
    pub struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    impl Controller {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &AnyObject) {
            self.with_state(|state, me| me.on_tick(state));
        }

        #[unsafe(method(accept:))]
        fn accept(&self, sender: &AnyObject) {
            let tagged = tag(sender);
            self.with_state(|state, _| {
                let index = showing(state, tagged);
                flash(state, &[index]);
                accept_one(state, index);
            });
        }

        #[unsafe(method(dismiss:))]
        fn dismiss(&self, sender: &AnyObject) {
            let tagged = tag(sender);
            self.with_state(|state, _| {
                let index = showing(state, tagged);
                if let Some(fix) = state.fixes.get(index) {
                    let original = fix.original(&state.text);
                    state.dismissed.insert((original, fix.replacement.clone()));
                }
                state.overlay.hide_card();
                refilter(state);
            });
        }

        #[unsafe(method(learn:))]
        fn learn(&self, sender: &AnyObject) {
            let tagged = tag(sender);
            self.with_state(|state, _| {
                let index = showing(state, tagged);
                if let Some(fix) = state.fixes.get(index) {
                    let word = fix.original(&state.text);
                    if let Err(err) = config::add_to_dictionary(&word) {
                        eprintln!("gramly: adding {word:?} to the dictionary: {err:#}");
                    }
                    state.dismissed.insert((word, fix.replacement.clone()));
                }
                state.overlay.hide_card();
                refilter(state);
                // Re-check so the other places that word was flagged clear too.
                state.submitted = false;
                state.changed_at = Instant::now() - PAUSE;
            });
        }

        #[unsafe(method(acceptSentence:))]
        fn accept_sentence(&self, sender: &AnyObject) {
            let tagged = tag(sender);
            self.with_state(|state, _| {
                let index = showing(state, tagged);
                let indices: Vec<usize> =
                    group(state, index).into_iter().map(|(i, _)| i).collect();
                flash(state, &indices);
                accept_fixes(state, &indices);
            });
        }

        #[unsafe(method(dismissSentence:))]
        fn dismiss_sentence(&self, sender: &AnyObject) {
            let tagged = tag(sender);
            self.with_state(|state, _| {
                let index = showing(state, tagged);
                for (_, fix) in group(state, index) {
                    let original = fix.original(&state.text);
                    state.dismissed.insert((original, fix.replacement.clone()));
                }
                state.overlay.hide_card();
                refilter(state);
            });
        }

        #[unsafe(method(acceptAll:))]
        fn accept_all(&self, _sender: &AnyObject) {
            self.with_state(accept_every);
        }

        #[unsafe(method(toggleEnabled:))]
        fn toggle_enabled(&self, _sender: &AnyObject) {
            update_config(|c| c.enabled = !c.enabled);
            self.with_state(|state, _| {
                reset(state);
                refresh_menu(state);
            });
        }

        #[unsafe(method(toggleAi:))]
        fn toggle_ai(&self, _sender: &AnyObject) {
            update_config(|c| c.ai = !c.ai);
            self.with_state(|state, _| {
                state.ai_error = None;
                reset(state);
                refresh_menu(state);
            });
        }

        #[unsafe(method(setKey:))]
        fn set_key(&self, _sender: &AnyObject) {
            ask_for_key(self.mtm());
            self.with_state(|state, _| {
                state.ai_error = None;
                reset(state);
                refresh_menu(state);
            });
        }

        #[unsafe(method(openDictionary:))]
        fn open_dictionary(&self, _sender: &AnyObject) {
            let path = config::dictionary_path();
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, "# One word per line. Gramly will never flag these.\n");
            }
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            NSWorkspace::sharedWorkspace().openURL(&url);
        }

        #[unsafe(method(grantAccess:))]
        fn grant_access(&self, _sender: &AnyObject) {
            ax::trusted(true);
            let link = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
            if let Some(url) = NSURL::URLWithString(&NSString::from_str(link)) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &AnyObject) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

fn tag(sender: &AnyObject) -> usize {
    let tag: isize = unsafe { msg_send![sender, tag] };
    tag.max(0) as usize
}

fn update_config(change: impl FnOnce(&mut Config)) {
    let mut config = Config::load().unwrap_or_default();
    change(&mut config);
    if let Err(err) = config.save() {
        eprintln!("gramly: saving settings: {err:#}");
    }
}

impl Controller {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            state: RefCell::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn with_state(&self, f: impl FnOnce(&mut State, &Self)) {
        // A re-entrant call (AppKit calling back into an action while another
        // holds the state) is dropped rather than panicking the app; the next
        // tick sees the same world again.
        let Ok(mut guard) = self.ivars().state.try_borrow_mut() else {
            return;
        };
        if let Some(state) = guard.as_mut() {
            f(state, self);
        }
    }

    fn target(&self) -> &AnyObject {
        let object: &NSObject = self.as_super();
        object
    }

    fn on_tick(&self, state: &mut State) {
        let mtm = self.mtm();
        let config = Config::load().unwrap_or_default();
        let trusted = ax::trusted(false);
        if trusted != state.trusted {
            state.trusted = trusted;
            refresh_menu(state);
        }
        if !config.enabled || !trusted {
            reset(state);
            return;
        }

        // Chromium and Electron build their accessibility tree only for an
        // app that asks; ask each app once, when it first comes forward.
        if let Some(pid) = ax::frontmost_pid()
            && state.woken.insert(pid)
        {
            ax::wake(pid);
        }

        // Focus.
        let field = ax::focused_field();
        let same = matches!((&field, &state.field), (Some(a), Some(b)) if a == b);
        if !same {
            reset(state);
            state.field = field;
        }
        let Some(field) = &state.field else {
            return;
        };

        // Text.
        let Some(text) = field.value() else {
            state.overlay.hide_all();
            return;
        };
        if text != state.text {
            // Carried across the edit rather than cleared: the underlines the
            // edit did not touch stay where they are, so typing and accepting
            // never make the rest flicker.
            state.local = rebase(&state.text, &text, &state.local);
            state.ai = rebase(&state.text, &text, &state.ai);
            state.fixes = rebase(&state.text, &text, &state.fixes);
            state.text = text;
            state.ai_current = false;
            state.changed_at = Instant::now();
            state.submitted = false;
            state.generation += 1;
            state.overlay.hide_card();
        }
        if !state.submitted && state.changed_at.elapsed() >= PAUSE {
            state.submitted = true;
            state.worker.submit(state.generation, state.text.clone());
        }

        // Answers.
        let mut changed = false;
        while let Some(outcome) = state.worker.try_recv() {
            match outcome {
                Outcome::Local { generation, fixes } if generation == state.generation => {
                    state.local = fixes;
                    changed = true;
                }
                Outcome::Ai { generation, fixes } if generation == state.generation => {
                    state.ai = fixes;
                    state.ai_current = true;
                    state.ai_error = None;
                    changed = true;
                }
                Outcome::AiFailed { reason, .. } if state.ai_error.as_deref() != Some(&reason) => {
                    eprintln!("gramly: model check failed: {reason}");
                    state.ai_error = Some(reason);
                    refresh_menu(state);
                }
                _ => {}
            }
        }
        if changed {
            refilter(state);
        }
        if state.fixes.is_empty() {
            state.overlay.hide_all();
            return;
        }

        // Geometry, every tick: the field scrolls and its window moves without
        // the text changing.
        let Some(frame) = state.field.as_ref().and_then(|f| f.visible_frame()) else {
            state.overlay.hide_all();
            return;
        };
        let field_rect = overlay::to_cocoa(frame, mtm);
        state.words.clear();
        for (index, fix) in state.fixes.iter().enumerate() {
            let (location, length) = fix.utf16(&state.text);
            // An insertion has no width of its own; underline the character
            // it follows so there is something to hover.
            let (location, length) = if length == 0 {
                (location.saturating_sub(1), 1)
            } else {
                (location, length)
            };
            let Some(bounds) = state
                .field
                .as_ref()
                .and_then(|f| f.bounds(location, length))
            else {
                continue;
            };
            let word = overlay::to_cocoa(bounds, mtm);
            // Scrolled out of the field's visible area: not drawn.
            let visible = word.origin.y >= field_rect.origin.y - 2.0
                && word.origin.y + word.size.height
                    <= field_rect.origin.y + field_rect.size.height + 2.0;
            if visible {
                state.words.push((index, word));
            }
        }

        // Hover.
        if let (Some(word), Some(index)) = (state.card_stale.take(), state.overlay.card_for) {
            state.overlay.hide_card();
            open_card(state, index, word, self);
        }
        let mouse = NSEvent::mouseLocation();
        state.overlay.hover(mouse);
        if state.overlay.card_for.is_none() {
            state.card_fix = None;
            state.card_word = None;
        }
        if in_corridor(state, mouse) {
            // On the card, its word, or the way between: nothing closes and
            // nothing under the card can steal the hover.
            state.left_at = None;
            state.hovering = None;
        } else {
            let over_word = state
                .words
                .iter()
                .find(|(_, rect)| overlay::contains(grow(*rect, 2.0), mouse))
                .copied();
            match over_word {
                Some((index, word)) => {
                    state.left_at = None;
                    let since = match state.hovering {
                        Some((current, since)) if current == index => since,
                        _ => {
                            let now = Instant::now();
                            state.hovering = Some((index, now));
                            now
                        }
                    };
                    if since.elapsed() >= HOVER {
                        open_card(state, index, word, self);
                    }
                }
                None => {
                    state.hovering = None;
                    if state.overlay.card_for.is_some() {
                        let left = *state.left_at.get_or_insert_with(Instant::now);
                        if left.elapsed() >= LEAVE {
                            state.overlay.hide_card();
                            state.left_at = None;
                        }
                    }
                }
            }
        }

        // Draw.
        // Everything the open card would change is lit, so a sentence card
        // shows which words "Correct the sentence" touches.
        let highlighted: Vec<usize> = state
            .overlay
            .card_for
            .map(|i| group(state, i).into_iter().map(|(i, _)| i).collect())
            .unwrap_or_default();
        let marks: Vec<Mark> = state
            .words
            .iter()
            .map(|(index, rect)| Mark {
                rect: *rect,
                kind: state.fixes[*index].kind,
                highlighted: highlighted.contains(index),
                flash: false,
            })
            .collect();
        let mut marks = marks;
        if let Some((rects, since)) = &state.flashing {
            if since.elapsed() < FLASH {
                marks.extend(rects.iter().map(|rect| Mark {
                    rect: *rect,
                    kind: gramly_core::Kind::Correctness,
                    highlighted: true,
                    flash: true,
                }));
            } else {
                state.flashing = None;
            }
        }
        let area = grow(field_rect, 4.0);
        state.overlay.marks(area, marks);
        let target = self.target();
        state.overlay.pill(field_rect, state.fixes.len(), target);
    }
}

/// Every fix in the same sentence as `index`, in order. One means the word
/// card; more means the sentence card.
fn group(state: &State, index: usize) -> Vec<(usize, Fix)> {
    let Some(fix) = state.fixes.get(index) else {
        return Vec::new();
    };
    let (start, end) = sentence(&state.text, fix.start);
    state
        .fixes
        .iter()
        .enumerate()
        .filter(|(_, f)| f.start >= start && f.end <= end.max(start))
        .map(|(i, f)| (i, f.clone()))
        .collect()
}

fn grow(rect: CGRect, by: f64) -> CGRect {
    CGRect::new(
        CGPoint::new(rect.origin.x - by, rect.origin.y - by),
        CGSize::new(rect.size.width + by * 2.0, rect.size.height + by * 2.0),
    )
}

fn reset(state: &mut State) {
    state.field = None;
    state.text.clear();
    state.local.clear();
    state.ai.clear();
    state.fixes.clear();
    state.words.clear();
    state.dismissed.clear();
    state.hovering = None;
    state.left_at = None;
    state.submitted = false;
    state.ai_current = false;
    state.generation += 1;
    state.overlay.hide_all();
}

fn refilter(state: &mut State) {
    let text = &state.text;
    let dismissed = &state.dismissed;
    // The model read the sentence; Harper matched patterns. Harper is the
    // instant first draw and the offline fallback, and on French slang it
    // offered "Eat" for "ERT" and "ma's" for "mais" - so once the model has
    // answered for this text, what it did not flag is not flagged.
    let base = if state.ai_current {
        state.ai.clone()
    } else {
        merge(state.local.clone(), state.ai.clone())
    };
    state.fixes = base
        .into_iter()
        .filter(|fix| !dismissed.contains(&(fix.original(text), fix.replacement.clone())))
        .collect();

    // The open card follows its fix through the rebuild: same words and same
    // replacement, it stays exactly as it is; same words with a better
    // replacement (the model's, after Harper's), it changes in place; gone,
    // it closes. Closing it unconditionally is what made a card shown for
    // "ok" vanish a second later when the model's answer arrived.
    let (Some(shown), Some(word)) = (state.card_fix.clone(), state.card_word) else {
        state.overlay.hide_card();
        return;
    };
    match state
        .fixes
        .iter()
        .position(|f| f.start == shown.start && f.end == shown.end)
    {
        Some(index) if state.fixes[index] == shown => state.overlay.card_for = Some(index),
        Some(index) => {
            // Redrawn on the next tick, 50ms on; the content changes, the
            // card does not blink away.
            state.overlay.card_for = Some(index);
            state.card_fix = Some(state.fixes[index].clone());
            state.card_stale = Some(word);
        }
        None => state.overlay.hide_card(),
    }
}

/// The fix an action is about: the one the card is showing, not the index
/// baked into the button when the card was drawn - the list can have been
/// rebuilt since.
fn showing(state: &State, tagged: usize) -> usize {
    state.overlay.card_for.unwrap_or(tagged)
}

fn open_card(state: &mut State, index: usize, word: CGRect, controller: &Controller) {
    let members = group(state, index);
    state.card_fix = state.fixes.get(index).cloned();
    state.card_word = Some(word);
    state
        .overlay
        .show_card(index, &members, &state.text, word, controller.target());
}

/// The card, its word, and the band between them, so the pointer can travel
/// from word to card at any speed and by any route without the card closing
/// and without a word lying under the card taking over.
fn in_corridor(state: &State, mouse: CGPoint) -> bool {
    let (Some(card), Some(word)) = (state.overlay.card_frame(), state.card_word) else {
        return false;
    };
    if overlay::contains(card, mouse) || overlay::contains(grow(word, 3.0), mouse) {
        return true;
    }
    let left = card.origin.x.min(word.origin.x) - 4.0;
    let right = (card.origin.x + card.size.width).max(word.origin.x + word.size.width) + 4.0;
    let card_top = card.origin.y + card.size.height;
    let (low, high) = if card_top <= word.origin.y {
        (card_top, word.origin.y)
    } else {
        (word.origin.y + word.size.height, card.origin.y)
    };
    mouse.x >= left && mouse.x <= right && mouse.y >= low - 2.0 && mouse.y <= high + 2.0
}

/// Starts the green glow on the words about to be accepted.
fn flash(state: &mut State, indices: &[usize]) {
    let rects: Vec<CGRect> = state
        .words
        .iter()
        .filter(|(i, _)| indices.contains(i))
        .map(|(_, rect)| *rect)
        .collect();
    if !rects.is_empty() {
        state.flashing = Some((rects, Instant::now()));
    }
}

/// One fix, through Accessibility if the app honours it and as a paste over a
/// selection if it does not.
fn accept_one(state: &mut State, index: usize) {
    let Some(fix) = state.fixes.get(index).cloned() else {
        return;
    };
    let Some(field) = &state.field else {
        return;
    };
    let expected = apply(&state.text, std::slice::from_ref(&fix));
    let (location, length) = fix.utf16(&state.text);
    if !field.replace(location, length, &fix.replacement, &expected)
        && field.select(location, length)
    {
        paste::paste(&fix.replacement);
    }
    state.overlay.hide_card();
}

/// Every fix in the field.
fn accept_every(state: &mut State, _: &Controller) {
    let all: Vec<usize> = (0..state.fixes.len()).collect();
    flash(state, &all);
    accept_fixes(state, &all);
}

/// Several fixes at once, back to front so each write leaves the offsets of
/// the rest valid. If the app refuses direct writes, the span they cover is
/// pasted once, corrected, over a selection of the same span - a paste per
/// fix would race the clipboard, and a select-all would reformat text the
/// user never asked to touch.
fn accept_fixes(state: &mut State, indices: &[usize]) {
    let Some(field) = &state.field else {
        return;
    };
    let mut fixes: Vec<Fix> = indices
        .iter()
        .filter_map(|i| state.fixes.get(*i).cloned())
        .collect();
    fixes.sort_by_key(|f| f.start);
    let (Some(lo), Some(hi)) = (
        fixes.iter().map(|f| f.start).min(),
        fixes.iter().map(|f| f.end).max(),
    ) else {
        return;
    };
    let text = state.text.clone();
    let corrected = apply(&text, &fixes);
    let mut current = text.clone();
    let mut direct = true;
    for fix in fixes.iter().rev() {
        let next = apply(&current, std::slice::from_ref(fix));
        let (location, length) = fix.utf16(&current);
        if !field.replace(location, length, &fix.replacement, &next) {
            direct = false;
            break;
        }
        current = next;
    }
    if !direct {
        // Every change lies inside [lo, hi), so outside it `current`, `text`
        // and `corrected` agree and only the span's end moves.
        let len = |s: &str| s.chars().count() as isize;
        let current_hi = (hi as isize + len(&current) - len(&text)) as usize;
        let corrected_hi = (hi as isize + len(&corrected) - len(&text)) as usize;
        let span = Fix {
            start: lo,
            end: current_hi,
            replacement: String::new(),
            kind: fixes[0].kind,
            message: String::new(),
            source: fixes[0].source,
        };
        let (location, length) = span.utf16(&current);
        let replacement: String = corrected.chars().skip(lo).take(corrected_hi - lo).collect();
        if field.select(location, length) {
            paste::paste(&replacement);
        }
    }
    state.overlay.hide_card();
}

fn refresh_menu(state: &mut State) {
    let config = Config::load().unwrap_or_default();
    let status = if !state.trusted {
        "Needs Accessibility access".to_string()
    } else if !config.enabled {
        "Paused".to_string()
    } else if !config.ai || config.openrouter_key.is_none() {
        "Checking on this Mac only".to_string()
    } else if let Some(error) = &state.ai_error {
        format!("Model unavailable: {}", truncate(error, 48))
    } else {
        "Checking with Claude Haiku 5.5".to_string()
    };
    if let Some(item) = &state.menu_status {
        item.setTitle(&NSString::from_str(&status));
    }
    if let Some(item) = &state.menu_enabled {
        item.setState(config.enabled as isize);
    }
    if let Some(item) = &state.menu_ai {
        item.setState((config.ai && config.openrouter_key.is_some()) as isize);
    }
    if let Some(item) = &state.menu_access {
        item.setHidden(state.trusted);
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max - 1).collect();
    format!("{cut}…")
}

fn ask_for_key(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("OpenRouter key"));
    alert.setInformativeText(&NSString::from_str(
        "Paste a key from openrouter.ai/keys. It is stored in ~/.config/gramly/config.toml, readable only by you. Leave it empty to check on this Mac only.",
    ));
    alert.addButtonWithTitle(&NSString::from_str("Save"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let input = NSSecureTextField::initWithFrame(
        NSSecureTextField::alloc(mtm),
        CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(300.0, 24.0)),
    );
    let current = Config::load().unwrap_or_default();
    if let Some(key) = &current.openrouter_key {
        input.setStringValue(&NSString::from_str(key));
    }
    alert.setAccessoryView(Some(&input));
    alert.window().setInitialFirstResponder(Some(&input));
    if alert.runModal() == NSAlertFirstButtonReturn {
        let key = input.stringValue().to_string().trim().to_string();
        if !key.is_empty() && !gramly_core::router::valid_key(&key) {
            let warning = NSAlert::new(mtm);
            warning.setMessageText(&NSString::from_str(
                "That does not look like an OpenRouter key",
            ));
            warning.runModal();
            return;
        }
        update_config(|c| c.openrouter_key = Some(key).filter(|k| !k.is_empty()));
    }
}

fn menu_item(
    title: &str,
    action: Option<objc2::runtime::Sel>,
    key: &str,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    };
    if action.is_some() {
        unsafe { item.setTarget(Some(target)) };
    } else {
        item.setEnabled(false);
    }
    item
}

fn build_menu(state: &mut State, controller: &Controller, mtm: MainThreadMarker) {
    let target = controller.target();
    let bar = NSStatusBar::systemStatusBar();
    let item = bar.statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("text.badge.checkmark"),
            Some(&NSString::from_str("Gramly")),
        );
        match image {
            Some(image) => {
                image.setTemplate(true);
                button.setImage(Some(&image));
            }
            None => button.setTitle(&NSString::from_str("G")),
        }
    }

    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    let status = menu_item("Starting…", None, "", target, mtm);
    let enabled = menu_item(
        "Check as I type",
        Some(sel!(toggleEnabled:)),
        "",
        target,
        mtm,
    );
    let ai = menu_item("AI suggestions", Some(sel!(toggleAi:)), "", target, mtm);
    let key = menu_item("OpenRouter key…", Some(sel!(setKey:)), "", target, mtm);
    let dictionary = menu_item(
        "Personal dictionary…",
        Some(sel!(openDictionary:)),
        "",
        target,
        mtm,
    );
    let access = menu_item(
        "Grant Accessibility access…",
        Some(sel!(grantAccess:)),
        "",
        target,
        mtm,
    );
    let about = menu_item(&format!("Gramly {VERSION}"), None, "", target, mtm);
    let quit = menu_item("Quit Gramly", Some(sel!(quit:)), "q", target, mtm);

    menu.addItem(&status);
    menu.addItem(&access);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&enabled);
    menu.addItem(&ai);
    menu.addItem(&key);
    menu.addItem(&dictionary);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&about);
    menu.addItem(&quit);
    item.setMenu(Some(&menu));

    state.status = Some(item);
    state.menu_status = Some(status);
    state.menu_enabled = Some(enabled);
    state.menu_ai = Some(ai);
    state.menu_access = Some(access);
    refresh_menu(state);
}

/// ⌘V, ⌘C, ⌘X and ⌘A are menu key equivalents, not text-field behaviour: an
/// app with no Edit menu cannot paste into its own key dialog. Gramly shows
/// no menu bar, but the menu still has to exist for the shortcuts to route.
fn edit_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let main = NSMenu::new(mtm);
    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
    for (title, action, key) in [
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        // No target: the action goes to whatever has focus, the key field.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        edit.addItem(&item);
    }
    let holder = NSMenuItem::new(mtm);
    holder.setSubmenu(Some(&edit));
    main.addItem(&holder);
    main
}

pub fn run() -> anyhow::Result<()> {
    let mtm = MainThreadMarker::new().ok_or_else(|| anyhow::anyhow!("not on the main thread"))?;
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon and no menu bar of its own: Gramly lives in the status
    // item and on top of other apps' text.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.setMainMenu(Some(&edit_menu(mtm)));

    // Asks once, the first time; macOS remembers the answer per signed binary.
    let trusted = ax::trusted(true);

    let controller = Controller::new(mtm);
    let mut state = State {
        worker: Worker::spawn(),
        overlay: Overlay::new(mtm),
        field: None,
        text: String::new(),
        changed_at: Instant::now(),
        generation: 0,
        submitted: false,
        local: Vec::new(),
        ai: Vec::new(),
        fixes: Vec::new(),
        dismissed: HashSet::new(),
        words: Vec::new(),
        hovering: None,
        left_at: None,
        card_fix: None,
        card_word: None,
        flashing: None,
        card_stale: None,
        woken: HashSet::new(),
        ai_error: None,
        ai_current: false,
        trusted,
        status: None,
        menu_status: None,
        menu_enabled: None,
        menu_ai: None,
        menu_access: None,
    };
    build_menu(&mut state, &controller, mtm);
    *controller.ivars().state.borrow_mut() = Some(state);

    let _timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            TICK,
            controller.target(),
            sel!(tick:),
            None,
            true,
        )
    };
    eprintln!(
        "gramly {VERSION}: ready{}",
        if trusted {
            ""
        } else {
            " (waiting for Accessibility access)"
        }
    );
    app.run();
    Ok(())
}
