//! macOS as the engine asks about it: Accessibility for the focused field,
//! the pasteboard and keystrokes for writing, `NSEvent` for the pointer.

use crate::macos::{ax, paste, primary_height};
use crate::surface::write::Paste;
use crate::surface::{Focus, Host, Point};
use gramly_core::Config;
use objc2::MainThreadMarker;
use objc2_app_kit::NSEvent;
use objc2_foundation::NSString;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How often an app whose focus reads as no text field is asked again to
/// expose its text (see `ax::wake`).
const REWAKE: Duration = Duration::from_secs(5);

pub struct MacHost {
    mtm: MainThreadMarker,
    /// When each app was last asked to expose its text (see `ax::wake`).
    woken: HashMap<i32, Instant>,
    /// The process in front last tick, Gramly included.
    front_pid: Option<i32>,
}

impl MacHost {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            mtm,
            woken: HashMap::new(),
            front_pid: None,
        }
    }
}

impl Host for MacHost {
    type Field = ax::Field;

    fn config(&self) -> Config {
        Config::load().unwrap_or_default()
    }

    fn trusted(&self) -> bool {
        ax::trusted(false)
    }

    fn front(&self) -> Option<(String, String)> {
        ax::frontmost_app()
    }

    fn focus(&mut self, current: Option<&ax::Field>) -> Focus<ax::Field> {
        let front_pid = ax::frontmost_pid();
        if front_pid != self.front_pid {
            self.front_pid = front_pid;
            if current.is_some_and(|field| Some(field.pid()) != front_pid) {
                return Focus::Elsewhere;
            }
        }
        // Chromium and Electron build their accessibility tree only for an
        // app that asks. Ask each app when it first comes forward, and again
        // whenever its focus reads as no text field: Slack dropped its tree
        // hours after the first ask and reported no focused element until it
        // was asked again.
        let Some(pid) = front_pid else {
            return ax::focus(current);
        };
        self.woken.entry(pid).or_insert_with(|| {
            ax::wake(pid);
            Instant::now()
        });
        let focus = ax::focus(current);
        if matches!(focus, Focus::Unknown | Focus::Elsewhere)
            && pid != std::process::id() as i32
            && self.woken[&pid].elapsed() >= REWAKE
        {
            ax::wake(pid);
            self.woken.insert(pid, Instant::now());
        }
        focus
    }

    fn pointer(&self) -> Point {
        let mouse = NSEvent::mouseLocation();
        Point {
            x: mouse.x,
            y: primary_height(self.mtm) - mouse.y,
        }
    }

    fn clipboard(&self) -> &dyn Paste {
        &paste::Pasteboard
    }

    fn undo(&self) {
        paste::undo();
    }

    fn clock(&self, at: Instant) -> String {
        use objc2_foundation::{NSDate, NSDateFormatter};
        let formatter = NSDateFormatter::new();
        formatter.setLocalizedDateFormatFromTemplate(&NSString::from_str("jmm"));
        let seconds = at.saturating_duration_since(Instant::now()).as_secs_f64();
        formatter
            .stringFromDate(&NSDate::dateWithTimeIntervalSinceNow(seconds))
            .to_string()
    }
}
