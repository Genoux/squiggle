//! Linux as the engine asks about it: AT-SPI for the focused field,
//! Hyprland's socket for the front window and the pointer.

use super::atspi::{self, Field, Focus, Source};
use super::clipboard;
use super::hypr::{self, Window};
use crate::surface::write::{Paste, Refusal};
use crate::surface::{self, Host, Point};
use gramly_core::Config;
use gtk4::gio::{AppInfo, prelude::AppInfoExt};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime};
use zbus::blocking::Connection;

/// How often a window with no known field is walked again, and how many
/// times. A Gecko window that has just come forward answers empty for a
/// moment, so one walk is not enough; but a walk is some 2000 calls into the
/// app (0.3s in a Zen window with no field focused), so a window with no
/// text field is not walked forever. A field focused later announces itself.
const WALK_EVERY: Duration = Duration::from_secs(1);
const WALKS: u8 = 3;

/// What the front window's focus resolved to.
enum Target {
    /// Nothing found yet.
    Unknown,
    /// A password field or a control that takes no text: nothing to look for.
    Other,
    Field(Source),
}

/// A walk running on its own thread: on the main loop, a 0.3s walk froze
/// the card and the hover with it. Dropping it stops the thread at its next
/// node, so a walk for a window nobody is in any more does not keep going.
struct Walking {
    answer: Receiver<Target>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Walking {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Walking {
    fn start(conn: Connection, window: Window) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, answer) = mpsc::channel();
        let stop = Arc::clone(&cancel);
        std::thread::spawn(move || {
            let at = Rc::new(Cell::new(window.at));
            let target = atspi::walk(&conn, &window, &at, &stop).map_or(Target::Unknown, classify);
            let _ = sender.send(target);
        });
        Self { answer, cancel }
    }
}

pub struct LinuxHost {
    conn: Connection,
    focus: Focus,
    /// The front window's top-left, read every tick and shared with the
    /// field: the window moves without the text changing.
    at: Rc<Cell<(f64, f64)>>,
    /// The last pointer position Hyprland answered.
    pointer: Cell<Point>,
    /// The focus event's source and the front window's address that `target`
    /// was decided from. Qualifying a field is three calls into another app,
    /// so it happens when either changes and not on every tick.
    decided_from: Option<(Option<Source>, String)>,
    /// The window `target` was found in. Keystrokes go to it and nowhere
    /// else, and only while it is still the front one.
    window: Option<Window>,
    target: Target,
    walked_at: Option<Instant>,
    walks_left: u8,
    walking: Option<Walking>,
    /// Display names by class: `front` runs every tick and a desktop entry
    /// lookup reads the disk.
    names: RefCell<HashMap<String, String>>,
}

/// The desktop entry's name, else the class's last segment capitalised
/// ("com.slack.Slack" -> "Slack"), for windows with no entry. gio 0.22 moved
/// `DesktopAppInfo` to a separate crate, so the entry is found among
/// `AppInfo::all`, matching its id to the class without regard to case.
fn display_name(class: &str) -> String {
    let wanted = format!("{class}.desktop");
    AppInfo::all()
        .into_iter()
        .find(|info| info.id().is_some_and(|id| id.eq_ignore_ascii_case(&wanted)))
        .map_or_else(|| fallback_name(class), |info| info.name().to_string())
}

fn fallback_name(class: &str) -> String {
    let last = class.rsplit('.').next().unwrap_or(class);
    let mut letters = last.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

impl LinuxHost {
    pub fn new(conn: Connection, focus: Focus) -> Self {
        Self {
            conn,
            focus,
            at: Rc::new(Cell::new((0.0, 0.0))),
            pointer: Cell::new(Point { x: 0.0, y: 0.0 }),
            decided_from: None,
            window: None,
            target: Target::Unknown,
            walked_at: None,
            walks_left: WALKS,
            walking: None,
            names: RefCell::default(),
        }
    }

    fn resolve_event(&self, source: Option<&Source>, pid: u32) -> Target {
        match source.and_then(|source| atspi::from_event(&self.conn, source, pid, &self.at)) {
            Some(Some(field)) => classify(field),
            Some(None) => Target::Other,
            None => Target::Unknown,
        }
    }

    /// Starts a walk, or takes the answer of the one running.
    fn walk(&mut self, window: &Window) {
        if let Some(walking) = &self.walking {
            match walking.answer.try_recv() {
                Ok(target) => self.target = target,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {}
            }
            self.walking = None;
            return;
        }
        let due = self.walked_at.is_none_or(|at| at.elapsed() >= WALK_EVERY);
        if !matches!(self.target, Target::Unknown) || self.walks_left == 0 || !due {
            return;
        }
        self.walks_left -= 1;
        self.walked_at = Some(Instant::now());
        self.walking = Some(Walking::start(self.conn.clone(), window.clone()));
    }

    /// Sends `mods`+`keycode` to the window the field was found in, if that
    /// is still the front window: Wayland gives the keystroke to whichever
    /// window Hyprland is told, and a fix pasted into the wrong one lands in
    /// somebody else's text.
    fn send_keys(&self, mods: &str, keycode: u32) -> Result<(), Refusal> {
        let window = self.window.as_ref().ok_or("no window for the field")?;
        let front = hypr::active_window();
        if !hypr::still_front(window, front.as_ref()) {
            note!(
                "keys not sent: {} is no longer the front window",
                window.class
            );
            return Err("the window changed before the paste");
        }
        if hypr::send_shortcut(mods, keycode, window) {
            Ok(())
        } else {
            note!("Hyprland did not take the shortcut for {}", window.class);
            Err("Hyprland did not take the shortcut")
        }
    }
}

fn classify(field: Field) -> Target {
    if field.is_password() {
        Target::Other
    } else {
        Target::Field(field.source())
    }
}

impl Host for LinuxHost {
    type Field = Field;

    fn config(&self) -> Config {
        Config::load().unwrap_or_default()
    }

    /// The accessibility bus is up and switched on before the host exists.
    fn trusted(&self) -> bool {
        true
    }

    /// The class is the id: Hyprland has no stable app id apart from it.
    fn front(&self) -> Option<(String, String)> {
        let class = hypr::active_window()?.class;
        let name = self
            .names
            .borrow_mut()
            .entry(class.clone())
            .or_insert_with(|| display_name(&class))
            .clone();
        Some((class, name))
    }

    /// Another window in front is an answer, not a miss: Hyprland reports it
    /// at once, and the field is given up without waiting out `GRACE`. So is
    /// no window (an empty workspace, a launcher holding the keyboard); a
    /// failed request costs one tick, as the next one finds the field again.
    fn focus(&mut self, current: Option<&Field>) -> surface::Focus<Field> {
        let Some(window) = hypr::active_window() else {
            return surface::Focus::Elsewhere;
        };
        self.at.set(window.at);
        let moved = self
            .window
            .as_ref()
            .is_some_and(|held| held.address != window.address);
        if atspi::terminal(&window.class) {
            return surface::Focus::Elsewhere;
        }
        let decided_from = (self.focus.latest(), window.address.clone());
        if self.decided_from.as_ref() != Some(&decided_from) {
            self.target = self.resolve_event(decided_from.0.as_ref(), window.pid);
            self.walked_at = None;
            self.walks_left = WALKS;
            // A walk still running was for another window or focus.
            self.walking = None;
            self.window = Some(window.clone());
            self.decided_from = Some(decided_from);
        }
        if moved && current.is_some() {
            return surface::Focus::Elsewhere;
        }
        self.walk(&window);
        match &self.target {
            Target::Field(source) => {
                let field = Field::from_source(&self.conn, source, &self.at);
                if current == Some(&field) {
                    surface::Focus::Same
                } else {
                    surface::Focus::Field(field)
                }
            }
            Target::Other => surface::Focus::Elsewhere,
            Target::Unknown => surface::Focus::Unknown,
        }
    }

    fn pointer(&self) -> Point {
        if let Some(pointer) = hypr::pointer() {
            self.pointer.set(pointer);
        }
        self.pointer.get()
    }

    fn clipboard(&self) -> &dyn Paste {
        self
    }

    fn undo(&self) {
        if let Err(why) = self.send_keys("CTRL", hypr::KEY_Z) {
            note!("undo not sent: {why}");
        }
    }

    fn clock(&self, at: Instant) -> String {
        crate::clock_time(SystemTime::now() + at.saturating_duration_since(Instant::now()))
    }
}

impl Paste for LinuxHost {
    fn stage(&self, text: &str) -> Result<(), Refusal> {
        clipboard::stage(text)
    }

    fn send(&self) -> Result<(), Refusal> {
        let sent = self.send_keys("CTRL", hypr::KEY_V);
        match sent {
            Ok(()) => clipboard::restore_after_paste(),
            Err(_) => clipboard::restore_now(),
        }
        sent
    }

    fn delete_selection(&self) -> Result<(), Refusal> {
        self.send_keys("", hypr::KEY_BACKSPACE)
    }

    fn unstage(&self) {
        clipboard::restore_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_class_with_no_entry_is_named_by_its_last_segment() {
        assert_eq!(fallback_name("com.slack.Slack"), "Slack");
        assert_eq!(fallback_name("orca"), "Orca");
        assert_eq!(fallback_name(""), "");
    }

    #[test]
    fn dropping_a_walk_cancels_it() {
        let (_sender, answer) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let walking = Walking {
            answer,
            cancel: Arc::clone(&cancel),
        };
        assert!(!cancel.load(Ordering::Relaxed));
        drop(walking);
        assert!(cancel.load(Ordering::Relaxed));
    }
}
