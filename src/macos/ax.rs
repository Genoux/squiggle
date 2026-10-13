//! The focused text field, through macOS Accessibility - the same API
//! Grammarly for Mac reads and writes through.
//!
//! Every call here is a synchronous IPC into another process. A hung app
//! would hang Gramly with it, so the system-wide element gets a short
//! messaging timeout and every caller treats "no answer" as "no field".

use objc2_app_kit::NSRunningApplication;
use objc2_application_services::{AXError, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{
    CFBoolean, CFDictionary, CFRange, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
};
use std::ffi::c_void;
use std::ptr::NonNull;

/// Long enough for a busy Electron app to answer, short enough that a hung
/// one costs a skipped frame rather than a frozen menu bar.
const MESSAGING_TIMEOUT: f32 = 0.15;

/// Apps whose "text area" is a terminal's scrollback. Correcting a shell
/// command is never what was meant, and Grammarly skips them for the same
/// reason.
const SKIPPED_APPS: &[&str] = &[
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "com.mitchellh.ghostty",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "com.github.wez.wezterm",
    "dev.warp.Warp-Stable",
    "co.zeit.hyper",
];

/// Roles that hold prose the user is typing. Secure fields are deliberately
/// absent: a password is never read, let alone sent anywhere.
const TEXT_ROLES: &[&str] = &["AXTextArea", "AXTextField", "AXComboBox", "AXSearchField"];

pub fn trusted(prompt: bool) -> bool {
    use objc2_application_services::{AXIsProcessTrustedWithOptions, kAXTrustedCheckOptionPrompt};
    let key: &CFString = unsafe { kAXTrustedCheckOptionPrompt };
    let value: &CFBoolean = CFBoolean::new(prompt);
    let options = CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[value]);
    unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) }
}

pub struct Field {
    element: CFRetained<AXUIElement>,
    /// Whether the field answers `AXBoundsForRange` itself; `None` until
    /// asked. Chromium's text areas answer with an empty rectangle at x = 0
    /// for every range, so they are measured through their text children.
    own_bounds: std::cell::Cell<Option<bool>>,
    /// The field's text children for the text they were read for: where each
    /// one starts in the field's value (UTF-16) and how long it is.
    segments: std::cell::RefCell<Option<(String, Vec<Segment>)>>,
}

struct Segment {
    start: usize,
    len: usize,
    element: CFRetained<AXUIElement>,
}

impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        // CFEqual on AXUIElements compares the remote object, not the wrapper.
        *self.element == *other.element
    }
}

fn attribute(element: &AXUIElement, name: &'static str) -> Option<CFRetained<CFType>> {
    let name = CFString::from_static_str(name);
    let mut value: *const CFType = std::ptr::null();
    let error = unsafe { element.copy_attribute_value(&name, NonNull::from(&mut value)) };
    if error != AXError::Success || value.is_null() {
        return None;
    }
    Some(unsafe { CFRetained::from_raw(NonNull::new_unchecked(value as *mut CFType)) })
}

fn string_attribute(element: &AXUIElement, name: &'static str) -> Option<String> {
    let value = attribute(element, name)?;
    value.downcast_ref::<CFString>().map(|s| s.to_string())
}

fn settable(element: &AXUIElement, name: &'static str) -> bool {
    let name = CFString::from_static_str(name);
    let mut result: u8 = 0;
    let error = unsafe { element.is_attribute_settable(&name, NonNull::from(&mut result).cast()) };
    error == AXError::Success && result != 0
}

fn point_of(value: &CFType) -> Option<CGPoint> {
    let value = value.downcast_ref::<AXValue>()?;
    let mut point = CGPoint::new(0.0, 0.0);
    let ok = unsafe {
        value.value(
            AXValueType::CGPoint,
            NonNull::from(&mut point).cast::<c_void>(),
        )
    };
    ok.then_some(point)
}

fn size_of(value: &CFType) -> Option<CGSize> {
    let value = value.downcast_ref::<AXValue>()?;
    let mut size = CGSize::new(0.0, 0.0);
    let ok = unsafe {
        value.value(
            AXValueType::CGSize,
            NonNull::from(&mut size).cast::<c_void>(),
        )
    };
    ok.then_some(size)
}

fn range_value(location: usize, length: usize) -> Option<CFRetained<AXValue>> {
    let mut range = CFRange::new(location as isize, length as isize);
    unsafe { AXValue::new(AXValueType::CFRange, NonNull::from(&mut range).cast()) }
}

fn bundle_id(pid: i32) -> String {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|app| app.bundleIdentifier())
        .map(|id| id.to_string())
        .unwrap_or_default()
}

pub fn frontmost_pid() -> Option<i32> {
    objc2_app_kit::NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
}

/// Asks an app to expose its text. Electron apps (Slack, Discord, VS Code,
/// Claude) and Chromium build no accessibility tree until an assistive app
/// sets `AXManualAccessibility` on them - which is what Grammarly does, and
/// why it showed in a Claude chat box where Gramly did not. Apps that do not
/// know the attribute ignore it.
pub fn wake(pid: i32) {
    let app = unsafe { AXUIElement::new_application(pid) };
    unsafe { app.set_messaging_timeout(MESSAGING_TIMEOUT) };
    let name = CFString::from_static_str("AXManualAccessibility");
    let yes: &CFBoolean = CFBoolean::new(true);
    unsafe { app.set_attribute_value(&name, yes) };
}

/// The text field that has keyboard focus, if it is one Gramly should read.
pub fn focused_field() -> Option<Field> {
    explain_focus().ok()
}

/// [`focused_field`], with the reason when there is none - for `gramly doctor`.
pub fn explain_focus() -> Result<Field, String> {
    let system = unsafe { AXUIElement::new_system_wide() };
    unsafe { system.set_messaging_timeout(MESSAGING_TIMEOUT) };
    let element = focused_element(&system)?;

    let mut pid: i32 = 0;
    if unsafe { element.pid(NonNull::from(&mut pid)) } != AXError::Success {
        return Err("the focused element has no process".into());
    }
    if pid == std::process::id() as i32 {
        return Err("focus is in Gramly itself".into());
    }
    let app = bundle_id(pid);
    if SKIPPED_APPS.contains(&app.as_str()) {
        return Err(format!("{app} is a terminal, skipped on purpose"));
    }

    let role = string_attribute(&element, "AXRole").unwrap_or_default();
    let subrole = string_attribute(&element, "AXSubrole").unwrap_or_default();
    if role == "AXSecureTextField" || subrole == "AXSecureTextField" {
        return Err("a password field, never read".into());
    }
    // Browsers and Electron report a contenteditable as an AXGroup or
    // AXWebArea that is still editable; the settable value is what says so.
    let editable = TEXT_ROLES.contains(&role.as_str())
        || settable(&element, "AXValue")
        || settable(&element, "AXSelectedText");
    if !editable {
        return Err(format!("{app}: the focused {role} is not editable text"));
    }
    unsafe { element.set_messaging_timeout(MESSAGING_TIMEOUT) };
    Ok(Field::new(element))
}

/// The focused element, asked of the system first and of the frontmost app
/// second. The system-wide query fails in some apps (it returns
/// `kAXErrorCannotComplete` while the app is busy or has just come forward)
/// where asking the app directly still answers.
fn focused_element(system: &AXUIElement) -> Result<CFRetained<AXUIElement>, String> {
    if let Some(element) =
        attribute(system, "AXFocusedUIElement").and_then(|v| v.downcast::<AXUIElement>().ok())
    {
        return Ok(element);
    }
    let front = objc2_app_kit::NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .ok_or("no frontmost app")?;
    let app = unsafe { AXUIElement::new_application(front.processIdentifier()) };
    unsafe { app.set_messaging_timeout(MESSAGING_TIMEOUT) };
    attribute(&app, "AXFocusedUIElement")
        .and_then(|v| v.downcast::<AXUIElement>().ok())
        .ok_or_else(|| {
            let name = front
                .bundleIdentifier()
                .map(|id| id.to_string())
                .unwrap_or_default();
            format!("{name} reports no focused element")
        })
}

impl Field {
    fn new(element: CFRetained<AXUIElement>) -> Self {
        Self {
            element,
            own_bounds: std::cell::Cell::new(None),
            segments: std::cell::RefCell::new(None),
        }
    }

    pub fn value(&self) -> Option<String> {
        string_attribute(&self.element, "AXValue")
    }

    /// The field's own rectangle, in Accessibility's coordinates: global,
    /// origin at the top-left of the primary display, y growing downwards.
    pub fn frame(&self) -> Option<CGRect> {
        let position = attribute(&self.element, "AXPosition")?;
        let size = attribute(&self.element, "AXSize")?;
        let origin = point_of(&position)?;
        let size = size_of(&size)?;
        Some(CGRect::new(origin, size))
    }

    /// The part of the field actually on screen. A long text view is as tall
    /// as its whole document (TextEdit reported 2977pt for a window a third of
    /// that), so its own frame cannot clip anything; the scroll area around it
    /// is the visible part. Walks up at most three parents to find one.
    pub fn visible_frame(&self) -> Option<CGRect> {
        let own = self.frame()?;
        let mut current = self.element.clone();
        for _ in 0..3 {
            let Some(parent) =
                attribute(&current, "AXParent").and_then(|v| v.downcast::<AXUIElement>().ok())
            else {
                break;
            };
            if string_attribute(&parent, "AXRole").as_deref() == Some("AXScrollArea") {
                let position = attribute(&parent, "AXPosition")?;
                let size = attribute(&parent, "AXSize")?;
                let area = CGRect::new(point_of(&position)?, size_of(&size)?);
                return Some(intersect(own, area));
            }
            current = parent;
        }
        Some(own)
    }

    /// Where a UTF-16 range is drawn, in the same coordinates as [`frame`].
    /// Asked of the field, or of the text child holding the range when the
    /// field cannot say. `None` and that fix is simply not drawn.
    pub fn bounds(&self, location: usize, length: usize) -> Option<CGRect> {
        if self.own_bounds.get() != Some(false) {
            let own = self.own(location, length);
            if self.own_bounds.get().is_none() {
                self.own_bounds.set(Some(own.is_some()));
            }
            if own.is_some() {
                return own;
            }
        }
        self.through_children(location, length)
    }

    fn own(&self, location: usize, length: usize) -> Option<CGRect> {
        let range = range_value(location, length)?;
        let name = CFString::from_static_str("AXBoundsForRange");
        let mut value: *const CFType = std::ptr::null();
        let error = unsafe {
            self.element.copy_parameterized_attribute_value(
                &name,
                &range,
                NonNull::from(&mut value),
            )
        };
        if error != AXError::Success || value.is_null() {
            return None;
        }
        let value = unsafe { CFRetained::from_raw(NonNull::new_unchecked(value as *mut CFType)) };
        let value = value.downcast_ref::<AXValue>()?;
        let mut rect = CGRect::default();
        let ok = unsafe { value.value(AXValueType::CGRect, NonNull::from(&mut rect).cast()) };
        (ok && rect.size.width > 0.5 && rect.size.height > 0.5).then_some(rect)
    }

    /// The range measured in the text child that holds it. Children are
    /// matched to the field's value in order, by their own text, so a
    /// paragraph break the field adds between them does not shift anything.
    fn through_children(&self, location: usize, length: usize) -> Option<CGRect> {
        let text = self.value()?;
        let stale = self
            .segments
            .borrow()
            .as_ref()
            .is_none_or(|(read_for, _)| *read_for != text);
        if stale {
            let segments = segments_of(&self.element, &text);
            *self.segments.borrow_mut() = Some((text, segments));
        }
        let cache = self.segments.borrow();
        let (_, segments) = cache.as_ref()?;
        let segment = segments
            .iter()
            .find(|s| location >= s.start && location < s.start + s.len.max(1))?;
        let local = location - segment.start;
        let length = length.min(segment.len - local).max(1);
        Field::new(segment.element.clone()).own(local, length)
    }

    /// Selects a UTF-16 range in the field.
    pub fn select(&self, location: usize, length: usize) -> bool {
        let Some(range) = range_value(location, length) else {
            return false;
        };
        let name = CFString::from_static_str("AXSelectedTextRange");
        unsafe { self.element.set_attribute_value(&name, &range) == AXError::Success }
    }

    /// Replaces a UTF-16 range the way typing would: select it, then set the
    /// selected text. Going through the selection rather than writing the
    /// whole `AXValue` keeps the app's undo stack and its cursor intact.
    ///
    /// Returns whether the field now reads as expected. Some apps (Chromium
    /// among them) accept the write and do nothing, so the result is
    /// verified rather than trusted.
    pub fn replace(&self, location: usize, length: usize, text: &str, expected: &str) -> bool {
        if !self.select(location, length) {
            return false;
        }
        let name = CFString::from_static_str("AXSelectedText");
        let value = CFString::from_str(text);
        let written = unsafe { self.element.set_attribute_value(&name, &value) };
        written == AXError::Success && self.value().as_deref() == Some(expected)
    }
}

fn intersect(a: CGRect, b: CGRect) -> CGRect {
    let x0 = a.origin.x.max(b.origin.x);
    let y0 = a.origin.y.max(b.origin.y);
    let x1 = (a.origin.x + a.size.width).min(b.origin.x + b.size.width);
    let y1 = (a.origin.y + a.size.height).min(b.origin.y + b.size.height);
    CGRect::new(
        CGPoint::new(x0, y0),
        CGSize::new((x1 - x0).max(0.0), (y1 - y0).max(0.0)),
    )
}

/// A raw description of whatever has focus, for `gramly doctor --raw`: the
/// app, role and every attribute name it offers, and nothing of its text.
pub fn describe_focus() -> String {
    let system = unsafe { AXUIElement::new_system_wide() };
    unsafe { system.set_messaging_timeout(MESSAGING_TIMEOUT) };
    let element = match focused_element(&system) {
        Ok(element) => element,
        Err(reason) => return reason,
    };
    let mut pid: i32 = 0;
    unsafe { element.pid(NonNull::from(&mut pid)) };
    let names = |parameterized: bool| -> Vec<String> {
        let mut array: *const objc2_core_foundation::CFArray = std::ptr::null();
        let error = unsafe {
            if parameterized {
                element.copy_parameterized_attribute_names(NonNull::from(&mut array))
            } else {
                element.copy_attribute_names(NonNull::from(&mut array))
            }
        };
        if error != AXError::Success || array.is_null() {
            return vec![format!("<{error:?}>")];
        }
        let array: CFRetained<objc2_core_foundation::CFArray> =
            unsafe { CFRetained::from_raw(NonNull::new_unchecked(array as *mut _)) };
        let count = array.count();
        (0..count)
            .filter_map(|i| {
                let item = unsafe { array.value_at_index(i) };
                let item = unsafe { &*(item as *const CFType) };
                item.downcast_ref::<CFString>().map(|s| s.to_string())
            })
            .collect()
    };
    let value = attribute(&element, "AXValue");
    let value_kind = match &value {
        None => "none".to_string(),
        Some(v) => match v.downcast_ref::<CFString>() {
            Some(s) => format!("string of {} chars", s.to_string().chars().count()),
            None => "not a string".to_string(),
        },
    };
    format!(
        "app {}\nrole {:?} / {:?}\nAXValue {value_kind}\nsettable AXValue {} / AXSelectedText {}\nattributes {:?}\nparameterized {:?}",
        bundle_id(pid),
        string_attribute(&element, "AXRole"),
        string_attribute(&element, "AXSubrole"),
        settable(&element, "AXValue"),
        settable(&element, "AXSelectedText"),
        names(false),
        names(true),
    )
}

/// The raw answer to AXBoundsForRange, for doctor.
pub fn probe_bounds(location: usize, length: usize) -> String {
    let Ok(field) = explain_focus() else {
        return "no field".into();
    };
    let Some(range) = range_value(location, length) else {
        return "no range".into();
    };
    let name = CFString::from_static_str("AXBoundsForRange");
    let mut value: *const CFType = std::ptr::null();
    let error = unsafe {
        field
            .element
            .copy_parameterized_attribute_value(&name, &range, NonNull::from(&mut value))
    };
    if error != AXError::Success || value.is_null() {
        return format!("error {error:?}");
    }
    let value = unsafe { CFRetained::from_raw(NonNull::new_unchecked(value as *mut CFType)) };
    let Some(value) = value.downcast_ref::<AXValue>() else {
        return "not an AXValue".into();
    };
    let mut rect = CGRect::default();
    let ok = unsafe { value.value(AXValueType::CGRect, NonNull::from(&mut rect).cast()) };
    format!("ok={ok} {rect:?}")
}

/// The focused element's descendants (two levels), their roles, text
/// lengths and what AXBoundsForRange says for their first characters.
pub fn probe_children() -> String {
    let Ok(field) = explain_focus() else {
        return "no field".into();
    };
    let mut out = String::new();
    let mut stack: Vec<(CFRetained<AXUIElement>, usize)> = children_of(&field.element)
        .into_iter()
        .map(|c| (c, 1))
        .collect();
    while let Some((child, depth)) = stack.pop() {
        let role = string_attribute(&child, "AXRole").unwrap_or_default();
        let len = string_attribute(&child, "AXValue").map(|v| v.chars().count());
        let frame = attribute(&child, "AXPosition")
            .and_then(|p| point_of(&p))
            .zip(attribute(&child, "AXSize").and_then(|s| size_of(&s)));
        let probe = Field::new(child.clone()).own(0, 2);
        out.push_str(&format!(
            "{}{role} value_len={len:?} frame={frame:?} bounds(0,2)={probe:?}\n",
            "  ".repeat(depth)
        ));
        if depth < 3 {
            stack.extend(children_of(&child).into_iter().map(|c| (c, depth + 1)));
        }
    }
    out
}

fn children_of(element: &AXUIElement) -> Vec<CFRetained<AXUIElement>> {
    let Some(list) = attribute(element, "AXChildren") else {
        return Vec::new();
    };
    let Some(array) = list.downcast_ref::<objc2_core_foundation::CFArray>() else {
        return Vec::new();
    };
    (0..array.count())
        .filter_map(|i| {
            let item = unsafe { &*(array.value_at_index(i) as *const CFType) };
            item.downcast_ref::<AXUIElement>()
                .map(|e| unsafe { CFRetained::retain(NonNull::from(e)) })
        })
        .collect()
}

/// The `AXStaticText` descendants of a field, in reading order, placed in
/// the field's value. Bounded: a field with thousands of nodes is not a chat
/// box, and walking it every edit would cost more than the underline is
/// worth.
fn segments_of(field: &AXUIElement, text: &str) -> Vec<Segment> {
    const MOST_NODES: usize = 400;
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut out = Vec::new();
    let mut cursor = 0;
    let mut visited = 0;
    let mut stack: Vec<CFRetained<AXUIElement>> = children_of(field).into_iter().rev().collect();
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > MOST_NODES {
            break;
        }
        if string_attribute(&node, "AXRole").as_deref() == Some("AXStaticText") {
            let Some(piece) = string_attribute(&node, "AXValue") else {
                continue;
            };
            let piece: Vec<u16> = piece.encode_utf16().collect();
            if piece.is_empty() {
                continue;
            }
            if let Some(found) = units[cursor..]
                .windows(piece.len())
                .position(|w| w == piece.as_slice())
            {
                let start = cursor + found;
                cursor = start + piece.len();
                out.push(Segment {
                    start,
                    len: piece.len(),
                    element: node,
                });
            }
            continue;
        }
        stack.extend(children_of(&node).into_iter().rev());
    }
    out
}
