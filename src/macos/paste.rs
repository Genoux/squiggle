//! The fallback write: select through Accessibility, paste with ⌘V.
//!
//! Chromium and Electron fields accept `AXSelectedTextRange` but ignore a
//! write to `AXSelectedText`, so for them a fix is a paste over a selection,
//! which every text field understands. The user's clipboard is put back
//! afterwards - Flow learned that a paste which eats the clipboard is a bug
//! report, not a detail.

use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventTapLocation};
use objc2_foundation::NSString;
use std::time::Duration;

/// `kVK_ANSI_V`.
const KEY_V: u16 = 9;

/// How long the target app gets to read the pasteboard before it is
/// restored. Measured against Chrome and Slack, which read on the next
/// run-loop turn; restoring sooner pasted the user's old clipboard instead.
const RESTORE_AFTER: Duration = Duration::from_millis(250);

pub fn paste(text: &str) {
    let board = NSPasteboard::generalPasteboard();
    let kind = unsafe { NSPasteboardTypeString };
    let saved = board.stringForType(kind);

    board.clearContents();
    board.setString_forType(&NSString::from_str(text), kind);

    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(None, KEY_V, down);
        CGEvent::set_flags(event.as_deref(), CGEventFlags::MaskCommand);
        CGEvent::post(CGEventTapLocation::HIDEventTap, event.as_deref());
    }

    let saved = saved.map(|s| s.to_string());
    std::thread::spawn(move || {
        std::thread::sleep(RESTORE_AFTER);
        let board = NSPasteboard::generalPasteboard();
        board.clearContents();
        if let Some(saved) = saved {
            board.setString_forType(&NSString::from_str(&saved), unsafe {
                NSPasteboardTypeString
            });
        }
    });
}
