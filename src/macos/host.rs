//! macOS as the engine asks about it: Accessibility for the focused field,
//! the pasteboard and keystrokes for writing, `NSEvent` for the pointer.

use crate::macos::{ax, paste, primary_height};
use crate::surface::write::Paste;
use crate::surface::{Focus, Host, Point};
use gramly_core::Config;
use objc2::MainThreadMarker;
use objc2_app_kit::NSEvent;
use objc2_foundation::NSString;
use std::collections::HashSet;
use std::time::Instant;

pub struct MacHost {
    mtm: MainThreadMarker,
    /// Apps already asked to expose their text (see `ax::wake`).
    woken: HashSet<i32>,
    /// The process in front last tick, Gramly included.
    front_pid: Option<i32>,
}

impl MacHost {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            mtm,
            woken: HashSet::new(),
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
        // app that asks; ask each app once, when it first comes forward.
        if let Some(pid) = front_pid
            && self.woken.insert(pid)
        {
            ax::wake(pid);
        }
        ax::focus(current)
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
