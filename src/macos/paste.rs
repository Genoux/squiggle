//! The fallback write: select through Accessibility, paste with ⌘V.
//!
//! Chromium and Electron fields accept `AXSelectedTextRange` but ignore a
//! write to `AXSelectedText`, so for them a fix is a paste over a selection,
//! which every text field understands. The user's clipboard is put back
//! afterwards - Flow learned that a paste which eats the clipboard is a bug
//! report, not a detail. All of it, every item in every type: saving only
//! the text lost a copied screenshot to the first fix accepted.

use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString};
use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventTapLocation};
use objc2_foundation::{NSArray, NSData, NSString};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

/// `kVK_ANSI_V`.
const KEY_V: u16 = 9;
/// `kVK_ANSI_Z`.
const KEY_Z: u16 = 6;
/// `kVK_RightArrow`.
const KEY_RIGHT: u16 = 124;
/// `kVK_LeftArrow`.
const KEY_LEFT: u16 = 123;
/// `kVK_Delete`, the key labelled delete: backspace.
const KEY_DELETE: u16 = 51;

/// How long the target app gets to read the pasteboard before it is
/// restored. Measured against Chrome and Slack, which read on the next
/// run-loop turn; restoring sooner pasted the user's old clipboard instead.
const RESTORE_AFTER: Duration = Duration::from_millis(250);

pub fn paste(text: &str) {
    let board = NSPasteboard::generalPasteboard();
    let kind = unsafe { NSPasteboardTypeString };
    let saved = save(&board);

    board.clearContents();
    board.setString_forType(&NSString::from_str(text), kind);

    command(KEY_V);

    std::thread::spawn(move || {
        std::thread::sleep(RESTORE_AFTER);
        restore(&NSPasteboard::generalPasteboard(), saved);
    });
}

/// Delete in the focused app: removes the selection. Every field
/// understands it, where an empty paste is ignored by some.
pub fn delete() {
    press(KEY_DELETE, CGEventFlags::empty());
}

/// ⌘Z in the focused app: takes back a fix that went wrong.
pub fn undo() {
    command(KEY_Z);
}

/// Where keystrokes go instead of the app in front: `gramly lab` points
/// them at its own test window, so an audit never types into the user's
/// apps. Zero, the default, is the app in front.
static TARGET: AtomicI32 = AtomicI32::new(0);

pub fn target(pid: i32) {
    TARGET.store(pid, Ordering::Relaxed);
}

/// Shift-Right in the focused app: extends the selection by one character,
/// for a selection the app cannot be told to end at a line's end.
pub fn extend_right() {
    press(KEY_RIGHT, CGEventFlags::MaskShift);
}

/// Left in the focused app: the caret one character back.
pub fn left() {
    press(KEY_LEFT, CGEventFlags::empty());
}

fn command(key: u16) {
    press(key, CGEventFlags::MaskCommand);
}

fn press(key: u16, flags: CGEventFlags) {
    let pid = TARGET.load(Ordering::Relaxed);
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(None, key, down);
        CGEvent::set_flags(event.as_deref(), flags);
        if pid == 0 {
            CGEvent::post(CGEventTapLocation::HIDEventTap, event.as_deref());
        } else {
            CGEvent::post_to_pid(pid, event.as_deref());
        }
    }
}

/// Each pasteboard item as its (type, bytes) pairs, plain data so it can
/// cross to the restoring thread.
pub(crate) type Saved = Vec<Vec<(String, Vec<u8>)>>;

pub(crate) fn save(board: &NSPasteboard) -> Saved {
    let Some(items) = board.pasteboardItems() else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| {
            item.types()
                .iter()
                .filter_map(|kind| {
                    let data = item.dataForType(&kind)?;
                    Some((kind.to_string(), data.to_vec()))
                })
                .collect()
        })
        .collect()
}

pub(crate) fn restore(board: &NSPasteboard, saved: Saved) {
    board.clearContents();
    let items: Vec<_> = saved
        .into_iter()
        .map(|pairs| {
            let item = NSPasteboardItem::new();
            for (kind, bytes) in pairs {
                item.setData_forType(&NSData::with_bytes(&bytes), &NSString::from_str(&kind));
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    if !items.is_empty() {
        board.writeObjects(&NSArray::from_retained_slice(&items));
    }
}
