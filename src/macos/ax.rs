//! The focused text field, through macOS Accessibility - the same API
//! Grammarly for Mac reads and writes through.
//!
//! Every call here is a synchronous IPC into another process. A hung app
//! would hang Squiggle with it, so the system-wide element gets a short
//! messaging timeout and every caller treats "no answer" as "no field".

use crate::surface::write::{classify, read_back, settle};
use crate::surface::{self, Field as _, Focus, Rect, Write};
use objc2_app_kit::NSRunningApplication;
use objc2_application_services::{AXError, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{
    CFBoolean, CFDictionary, CFRange, CFRetained, CFString, CFType, CFURL, CGPoint, CGRect, CGSize,
};
use squiggle_core::fix::utf16_range;
use squiggle_core::off::Place;
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
    "com.stablyai.orca",
];

/// Gecko browsers. They ignore `AXManualAccessibility` and build their tree
/// on `AXEnhancedUserInterface`, the attribute VoiceOver sets (see `wake`).
/// Only they are sent it: other apps slow their window animations under it.
const GECKO_APPS: &[&str] = &[
    "app.zen-browser.zen",
    "org.mozilla.firefox",
    "org.mozilla.firefoxdeveloperedition",
    "org.mozilla.nightly",
    "org.mozilla.librewolf",
    "net.waterfox.waterfox",
    "one.ablaze.floorp",
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
    /// The app it belongs to.
    pid: i32,
    /// Whether the field answers `AXBoundsForRange` itself; `None` until
    /// asked. Chromium's text areas answer with an empty rectangle at x = 0
    /// for every range, so they are measured through their text children.
    own_bounds: std::cell::Cell<Option<bool>>,
    /// The field's text children for the text they were read for: where each
    /// one starts in the field's value (UTF-16) and how long it is.
    segments: std::cell::RefCell<Option<(String, Vec<Segment>)>>,
    /// What the field may show while empty, read once: its placeholder,
    /// description and title.
    labels: std::cell::OnceCell<Vec<String>>,
    /// How the app counts the text last read, where that differs from the
    /// value: for each UTF-16 unit of the value, its offset in the app's
    /// count, or None for a unit the app does not count. None overall when
    /// the two agree or the app cannot say. Zen gives ChatGPT's "⌘
    /// AdCollection" chip icon a U+FFFC in its offsets but none in
    /// `AXValue`, so every range after it landed one character early
    /// ("anold" -> "an old" wrote "Pickan old"). Chrome is the other way
    /// round: the line breaks between paragraphs are in the value and not
    /// in its offsets, so every line after the first was off by one more.
    ///
    /// The flag says whether the offsets were checked against the app and
    /// held: Zen's list returns its bullets in `AXStringForRange` but does
    /// not count them, so a map built from that string is wrong, and
    /// underlines are then placed by the field's children instead.
    hidden: std::cell::RefCell<(String, Option<OffsetMap>, bool)>,
    /// The last `AXValue` of a web field, and it with its line breaks put
    /// back; see [`with_lines`](Self::with_lines).
    lines: std::cell::RefCell<Option<(String, String)>>,
    /// Whether the app honours a write to `AXSelectedText`; `None` until
    /// one is tried. Once it has ignored one, the rest go straight to the
    /// paste, so a write that lands late cannot be pasted a second time.
    direct: std::cell::Cell<Option<bool>>,
}

/// For each UTF-16 unit of a field's value, its offset in the app's own
/// count, or None for a unit the app does not count; see `Field::hidden`.
type OffsetMap = Vec<Option<usize>>;

/// Whether a field is part of a web page - a browser's, or an Electron
/// app's. Both Gecko and Chromium give their elements a DOM identity, and
/// native AppKit fields never do.
fn web(element: &AXUIElement) -> bool {
    attribute(element, "AXEditableAncestor").is_some()
        || attribute(element, "AXDOMIdentifier").is_some()
}

struct Segment {
    start: usize,
    len: usize,
    element: CFRetained<AXUIElement>,
}

/// A field's element alone; see [`surface::Field::handle`].
pub struct Handle(CFRetained<AXUIElement>);

impl PartialEq for Handle {
    fn eq(&self, other: &Self) -> bool {
        *self.0 == *other.0
    }
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
    settable_answer(element, name) == Some(true)
}

/// Whether `name` can be set, or None if the app did not say.
fn settable_answer(element: &AXUIElement, name: &'static str) -> Option<bool> {
    let name = CFString::from_static_str(name);
    let mut result: u8 = 0;
    let error = unsafe { element.is_attribute_settable(&name, NonNull::from(&mut result).cast()) };
    (error == AXError::Success).then_some(result != 0)
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

/// The host of the page a web field is on, from the nearest web area above
/// it. Electron apps have one too ("app.slack.com").
fn page_host(element: &AXUIElement) -> Option<String> {
    let mut at = attribute(element, "AXParent")?
        .downcast::<AXUIElement>()
        .ok()?;
    for _ in 0..60 {
        if string_attribute(&at, "AXRole").as_deref() == Some("AXWebArea") {
            let url = attribute(&at, "AXURL")?.downcast::<CFURL>().ok()?;
            return url.host_name().map(|host| host.to_string());
        }
        at = attribute(&at, "AXParent")?.downcast::<AXUIElement>().ok()?;
    }
    None
}

pub fn frontmost_pid() -> Option<i32> {
    objc2_app_kit::NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
}

/// The app in front as (bundle id, name), for the log. None while Squiggle
/// itself is in front: it comes forward only for its own dialogs.
pub fn frontmost_app() -> Option<(String, String)> {
    let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    if app.processIdentifier() == std::process::id() as i32 {
        return None;
    }
    let id = app.bundleIdentifier()?.to_string();
    let name = app
        .localizedName()
        .map(|n| n.to_string())
        .unwrap_or_else(|| id.clone());
    Some((id, name))
}

/// The title of the focused window of the app in front, for the lab's
/// report of what took the keyboard.
pub fn frontmost_window_title() -> Option<String> {
    let pid = frontmost_pid()?;
    let app = unsafe { AXUIElement::new_application(pid) };
    let window = attribute(&app, "AXFocusedWindow")?;
    let window = window.downcast_ref::<AXUIElement>()?;
    string_attribute(window, "AXTitle")
}

/// Asks an app to expose its text. Electron apps (Slack, Discord, VS Code,
/// Claude) and Chromium build no accessibility tree until an assistive app
/// sets `AXManualAccessibility` on them - which is what Grammarly does, and
/// why it showed in a Claude chat box where Squiggle did not. Apps that do not
/// know the attribute ignore it. Skipped apps are not asked: Orca, an
/// Electron terminal, stopped taking pasted images once Squiggle woke it.
///
/// Zen (Gecko) answers `AXManualAccessibility` as unsupported and dropped
/// its tree to the window's three buttons, reporting the window as focused
/// while a field in the page had it; set, `AXEnhancedUserInterface` brought
/// the page back (6 nodes to 416). It had worked while another app kept it
/// set.
pub fn wake(pid: i32) {
    let bundle = bundle_id(pid);
    if SKIPPED_APPS.contains(&bundle.as_str()) {
        return;
    }
    let app = unsafe { AXUIElement::new_application(pid) };
    unsafe { app.set_messaging_timeout(MESSAGING_TIMEOUT) };
    let name = if GECKO_APPS.contains(&bundle.as_str()) {
        CFString::from_static_str("AXEnhancedUserInterface")
    } else {
        CFString::from_static_str("AXManualAccessibility")
    };
    let yes: &CFBoolean = CFBoolean::new(true);
    unsafe { app.set_attribute_value(&name, yes) };
}

/// What has keyboard focus, `current` being the field read last tick. The
/// same element is recognised before any question about it, so a field in
/// use costs one round trip a tick, not five.
pub fn focus(current: Option<&Field>) -> Focus<Field> {
    match examine_focus(current) {
        Ok(None) => Focus::Same,
        Ok(Some(field)) => Focus::Field(field),
        Err((true, _)) => Focus::Elsewhere,
        Err((false, _)) => Focus::Unknown,
    }
}

/// The focused text field, with the reason when there is none - for `squiggle
/// doctor`.
pub fn explain_focus() -> Result<Field, String> {
    match examine_focus(None) {
        Ok(Some(field)) => Ok(field),
        Ok(None) => unreachable!("nothing to be the same as"),
        Err((_, why)) => Err(why),
    }
}

/// The focused field: None when it is `current`. The error says whether the
/// app answered (it is not a field) or did not (it may still be one).
fn examine_focus(current: Option<&Field>) -> Result<Option<Field>, (bool, String)> {
    let system = unsafe { AXUIElement::new_system_wide() };
    unsafe { system.set_messaging_timeout(MESSAGING_TIMEOUT) };
    let element = focused_element(&system).map_err(|why| (false, why))?;
    if current.is_some_and(|field| *field.element == *element) {
        return Ok(None);
    }

    let mut pid: i32 = 0;
    if unsafe { element.pid(NonNull::from(&mut pid)) } != AXError::Success {
        return Err((false, "the focused element has no process".into()));
    }
    if pid == std::process::id() as i32 {
        return Err((true, "focus is in Squiggle itself".into()));
    }
    let app = bundle_id(pid);
    if SKIPPED_APPS.contains(&app.as_str()) {
        return Err((true, format!("{app} is a terminal, skipped on purpose")));
    }

    // Every element has a role; none back means the app did not answer.
    let role = string_attribute(&element, "AXRole")
        .ok_or_else(|| (false, format!("{app}: the focused element did not answer")))?;
    let subrole = string_attribute(&element, "AXSubrole").unwrap_or_default();
    if role == "AXSecureTextField" || subrole == "AXSecureTextField" {
        return Err((true, "a password field, never read".into()));
    }
    // Browsers and Electron report a contenteditable as an AXGroup or
    // AXWebArea that is still editable; the settable value is what says so.
    if !TEXT_ROLES.contains(&role.as_str()) {
        let value = settable_answer(&element, "AXValue");
        let selected = settable_answer(&element, "AXSelectedText");
        if value != Some(true) && selected != Some(true) {
            let answered = value.is_some() && selected.is_some();
            return Err((
                answered,
                format!("{app}: the focused {role} is not editable text"),
            ));
        }
    }
    unsafe { element.set_messaging_timeout(MESSAGING_TIMEOUT) };
    Ok(Some(Field::new(element)))
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
    pub fn pid(&self) -> i32 {
        self.pid
    }

    fn new(element: CFRetained<AXUIElement>) -> Self {
        // Kept in the element itself: no question to the app.
        let mut pid: i32 = 0;
        unsafe { element.pid(NonNull::from(&mut pid)) };
        Self {
            element,
            pid,
            own_bounds: std::cell::Cell::new(None),
            segments: std::cell::RefCell::new(None),
            labels: std::cell::OnceCell::new(),
            hidden: std::cell::RefCell::new((String::new(), None, true)),
            lines: std::cell::RefCell::new(None),
            direct: std::cell::Cell::new(None),
        }
    }

    /// `value` with a line break wherever the field shows a new line and the
    /// value has none. Zen reports a box of three paragraphs as one run of
    /// text ("...we roll backThanks"), and Squiggle read "backThanks" as one
    /// word; its bulleted list as "• fix the llimit• add a tset". The field's
    /// children still say where each line is: every paragraph or list item
    /// is a group of its own. Worked out once per change of the value.
    fn with_lines(&self, value: String) -> String {
        if let Some((raw, rebuilt)) = &*self.lines.borrow()
            && *raw == value
        {
            return rebuilt.clone();
        }
        let rebuilt = add_line_breaks(&value, &blocks_of(&self.element));
        *self.lines.borrow_mut() = Some((value, rebuilt.clone()));
        rebuilt
    }

    /// A char range of the text [`value`](surface::Field::value) last read,
    /// as `(location, length)` in UTF-16 units.
    fn utf16(&self, start: usize, len: usize) -> (usize, usize) {
        utf16_range(&self.hidden.borrow().0, start, start + len)
    }

    /// A UTF-16 range of the value as the app counts it; see
    /// [`hidden`](Self::hidden). Uses the text [`value`](Self::value) last
    /// read, which every caller has just done.
    fn app_range(&self, location: usize, length: usize) -> (usize, usize) {
        let hidden = self.hidden.borrow();
        let Some(map) = &hidden.1 else {
            return (location, length);
        };
        // The range starts at its first counted unit and ends after its
        // last, so an object the app counts just before it stays outside.
        let after = |from: usize| {
            map.get(from..)
                .and_then(|rest| rest.iter().flatten().next().copied())
                .unwrap_or_else(|| map.iter().flatten().last().map_or(0, |&i| i + 1))
        };
        let start = after(location);
        let end = map[..(location + length).min(map.len())]
            .iter()
            .rev()
            .flatten()
            .next()
            .map_or(start, |&i| i + 1);
        (start, end.max(start) - start)
    }

    /// Whether the value has a line break right after `end`: the end of a
    /// line, which some apps cannot tell apart from the start of the next.
    fn ends_line(&self, end: usize) -> bool {
        let hidden = self.hidden.borrow();
        hidden.0.encode_utf16().nth(end) == Some(u16::from(b'\n'))
    }

    /// Whether the value has a line break right before `start`.
    fn starts_line(&self, start: usize) -> bool {
        start > 0 && self.ends_line(start - 1)
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

    /// Where a UTF-16 range is drawn, in the same coordinates as [`frame`].
    /// Asked of the field, or of the text child holding the range when the
    /// field cannot say. `None` and that fix is simply not drawn.
    pub fn bounds_utf16(&self, location: usize, length: usize) -> Option<CGRect> {
        if !self.hidden.borrow().2
            && let Some(rect) = self.through_children(location, length)
        {
            return Some(rect);
        }
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
        let (location, length) = self.app_range(location, length);
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

    /// Selects a UTF-16 range in the field; whether the app reports that
    /// exact range selected within `SETTLE`.
    /// Selects a UTF-16 range of the value [`value`](Self::value) last
    /// read, and confirms the app selected exactly that text.
    pub fn select_utf16(&self, location: usize, length: usize) -> bool {
        let value_location = location;
        let value_end = location + length;
        let original = {
            let hidden = self.hidden.borrow();
            let units: Vec<u16> = hidden.0.encode_utf16().collect();
            match units.get(location..location + length) {
                Some(span) => String::from_utf16_lossy(span),
                None => return false,
            }
        };
        let Some((location, length)) = self.locate(location, length, &original) else {
            note!(
                "{}: the text to fix was not found where the app counts it",
                self.app()
            );
            return false;
        };
        // A range that starts or ends a line, in a web field, is selected
        // with the keyboard. Web engines count no line break between
        // paragraphs, so the end of one line and the start of the next are
        // one offset, read as the next line by Chrome and as the previous
        // one by Zen: the selection took the break with it and the paste
        // joined the lines ("- fix the limit- add a test", "Hi team,The
        // deploy"), while `AXSelectedText` read only the word. The caret
        // goes one character in, Left takes it to the start, and
        // Shift-Right once per character selects the rest: the keyboard
        // never leaves the line.
        let touches = length > 0
            && web(&self.element)
            && (self.starts_line(value_location) || self.ends_line(value_end));
        let name = CFString::from_static_str("AXSelectedTextRange");
        if touches {
            let step = original.chars().next().map_or(1, char::len_utf16);
            let Some(caret) = range_value(location + step, 0) else {
                return false;
            };
            if unsafe { self.element.set_attribute_value(&name, &caret) } != AXError::Success
                || !settle(|| self.selected_range() == Some((location + step, 0)))
            {
                return false;
            }
            crate::macos::paste::left();
            for _ in 0..original.chars().count() {
                crate::macos::paste::extend_right();
            }
        } else {
            let Some(range) = range_value(location, length) else {
                return false;
            };
            if unsafe { self.element.set_attribute_value(&name, &range) } != AXError::Success {
                return false;
            }
        }
        // Confirmed, not trusted: whatever is selected is what the next
        // write or paste replaces, and an app that kept an older, larger
        // selection had a whole paragraph replaced by one word.
        if !settle(|| self.selected_range() == Some((location, length))) {
            return false;
        }
        if std::env::var_os("SQUIGGLE_TRACE").is_some() {
            let value = self.hidden.borrow().0.clone();
            note!(
                "select {original:?} at {location}+{length}: app string {:?}, selected {:?}, value {:?}",
                string_for_range(&self.element, 0, value.encode_utf16().count() + 4),
                string_attribute(&self.element, "AXSelectedText"),
                value
            );
        }
        // The range the app reports is only its own count. What it holds
        // must be the text being fixed: in a bulleted list a range one
        // character early was confirmed, and the paste ate the space before
        // the word (a fix to "llimit" in a list left the field a character
        // short, and the line's spacing with it).
        match string_attribute(&self.element, "AXSelectedText") {
            Some(selected) if selected != original => {
                note!(
                    "{}: selected {:?} where {:?} was expected",
                    self.app(),
                    selected,
                    original
                );
                false
            }
            _ => true,
        }
    }

    /// For `squiggle lab --probe`: how the app counts this field's text.
    pub fn probe(&self) -> String {
        let count = attribute(&self.element, "AXNumberOfCharacters").and_then(|v| {
            v.downcast_ref::<objc2_core_foundation::CFNumber>()
                .and_then(|n| n.as_i64())
        });
        let whole = count.and_then(|n| string_for_range(&self.element, 0, n as usize));
        format!(
            "AXNumberOfCharacters {count:?}, AXStringForRange(0, that) {whole:?}, selection {:?} {:?}",
            self.selected_range(),
            string_attribute(&self.element, "AXSelectedText")
        )
    }

    /// For `squiggle lab --probe`: the field's descendants, role and text.
    pub fn tree(&self) -> String {
        let mut out = String::new();
        let mut stack: Vec<(CFRetained<AXUIElement>, usize)> = children_of(&self.element)
            .into_iter()
            .rev()
            .map(|c| (c, 1))
            .collect();
        while let Some((node, depth)) = stack.pop() {
            let role = string_attribute(&node, "AXRole").unwrap_or_default();
            let value = string_attribute(&node, "AXValue");
            out.push_str(&format!("{}{role} {value:?}\n", "  ".repeat(depth)));
            if depth < 4 {
                stack.extend(children_of(&node).into_iter().rev().map(|c| (c, depth + 1)));
            }
        }
        out
    }

    /// For `squiggle lab --probe`: a value range as the app counts it, and
    /// what the app holds there.
    pub fn mapped(&self, location: usize, length: usize) -> String {
        let (at, len) = self.app_range(location, length);
        format!(
            "{location}+{length} -> {at}+{len} {:?}, map {:?}",
            string_for_range(&self.element, at, len),
            self.hidden.borrow().1.as_ref().map(|m| m.len())
        )
    }

    /// For `squiggle lab --probe`: sets the selection exactly as given, in the
    /// app's own offsets.
    pub fn select_raw(&self, location: usize, length: usize) {
        if let Some(range) = range_value(location, length) {
            let name = CFString::from_static_str("AXSelectedTextRange");
            unsafe { self.element.set_attribute_value(&name, &range) };
        }
    }

    /// Whether the app holds, at the offsets [`app_range`](Self::app_range)
    /// gives, the words `text` has: checked on a few words across it. True
    /// when the app cannot say, so nothing changes for an app without
    /// `AXStringForRange`.
    fn offsets_hold(&self, text: &str) -> bool {
        let units: Vec<u16> = text.encode_utf16().collect();
        let words: Vec<(usize, usize)> = {
            let mut out = Vec::new();
            let mut start = None;
            for (i, &u) in units.iter().enumerate() {
                let letter = char::from_u32(u32::from(u)).is_some_and(char::is_alphanumeric);
                match (letter, start) {
                    (true, None) => start = Some(i),
                    (false, Some(s)) => {
                        out.push((s, i - s));
                        start = None;
                    }
                    _ => {}
                }
            }
            if let Some(s) = start {
                out.push((s, units.len() - s));
            }
            out.retain(|&(_, len)| len >= 3);
            out
        };
        if words.is_empty() {
            return true;
        }
        let picks = [0, words.len() / 2, words.len() - 1];
        picks.iter().all(|&i| {
            let (at, len) = words[i];
            let (app_at, app_len) = self.app_range(at, len);
            match string_for_range(&self.element, app_at, app_len) {
                Some(found) => found.encode_utf16().eq(units[at..at + len].iter().copied()),
                None => true,
            }
        })
    }

    /// Where the app counts `original`, the value's text at `location`: the
    /// range [`app_range`](Self::app_range) gives when the app holds it
    /// there, otherwise the nearest offset within a few characters that
    /// does, each asked on its own (a longer string read at once may hold
    /// characters the app does not count, as Zen's list holds its bullets).
    /// None when the app can say and the text is nowhere near.
    fn locate(&self, location: usize, length: usize, original: &str) -> Option<(usize, usize)> {
        const REACH: usize = 24;
        let (start, length) = self.app_range(location, length);
        let want: Vec<u16> = original.encode_utf16().collect();
        if want.is_empty() {
            return Some((start, length));
        }
        let holds = |at: usize| {
            string_for_range(&self.element, at, want.len())
                .map(|s| s.encode_utf16().eq(want.iter().copied()))
        };
        match holds(start) {
            Some(true) => return Some((start, want.len())),
            // The app cannot say; the selection is checked once made.
            None if string_for_range(&self.element, start, 1).is_none() => {
                return Some((start, length));
            }
            _ => {}
        }
        let found = (1..=REACH)
            .flat_map(|d| [start.checked_sub(d), Some(start + d)])
            .flatten()
            .find(|&at| holds(at) == Some(true))?;
        note!(
            "{}: offsets differ from the value by {}; fixed by finding the text",
            self.app(),
            found as isize - start as isize
        );
        Some((found, want.len()))
    }

    /// The bundle id of the app the field belongs to, for the log.
    fn app(&self) -> String {
        let mut pid: i32 = 0;
        unsafe { self.element.pid(NonNull::from(&mut pid)) };
        bundle_id(pid)
    }

    /// The selection, as the app counts it.
    fn selected_range(&self) -> Option<(usize, usize)> {
        let value = attribute(&self.element, "AXSelectedTextRange")?;
        let value = value.downcast_ref::<AXValue>()?;
        let mut range = CFRange::new(0, 0);
        let ok = unsafe { value.value(AXValueType::CFRange, NonNull::from(&mut range).cast()) };
        ok.then_some((range.location as usize, range.length as usize))
    }

    /// Replaces a UTF-16 range the way typing would: select it, then set the
    /// selected text. Going through the selection rather than writing the
    /// whole `AXValue` keeps the app's undo stack and its cursor intact.
    ///
    /// `before` is the field's text now, `expected` its text after. Some
    /// apps (Chromium among them) accept the write and do nothing, so the
    /// result is read back rather than trusted - and only a field left
    /// exactly as it was counts as ignored. One that changed into anything
    /// else did take a write, and pasting the fix on top wrote it twice.
    pub fn replace_utf16(
        &self,
        location: usize,
        length: usize,
        text: &str,
        before: &str,
        expected: &str,
    ) -> Write {
        // Web fields are never written directly; the paste goes through the
        // page's own input handling. Zen applied a direct write late in one
        // case and, in another, replaced a whole sentence with one word
        // while reporting the right selection.
        if self.direct.get() == Some(false) || web(&self.element) {
            self.direct.set(Some(false));
            return Write::Ignored;
        }
        if !self.select_utf16(location, length) {
            return Write::Unsafe;
        }
        let name = CFString::from_static_str("AXSelectedText");
        let value = CFString::from_str(text);
        let written = unsafe { self.element.set_attribute_value(&name, &value) };
        let after = if written == AXError::Success {
            read_back(|| self.value(), expected)
        } else {
            self.value()
        };
        let outcome = classify(before, after.as_deref(), expected);
        if outcome != Write::Unsafe {
            self.direct.set(Some(outcome == Write::Done));
        }
        outcome
    }
}

impl Field {
    /// Its placeholder, description and title, those it has, read once.
    fn labels(&self) -> &[String] {
        self.labels.get_or_init(|| {
            ["AXPlaceholderValue", "AXDescription", "AXTitle"]
                .into_iter()
                .filter_map(|name| string_attribute(&self.element, name))
                .map(|label| label.trim().to_string())
                .filter(|label| !label.is_empty())
                .collect()
        })
    }
}

impl surface::Field for Field {
    type Handle = Handle;

    fn handle(&self) -> Handle {
        Handle(self.element.clone())
    }

    /// The app, the page's host for a web field, its id and its first
    /// label.
    fn place(&self) -> Place {
        let app = bundle_id(self.pid);
        let site = if web(&self.element) {
            page_host(&self.element).unwrap_or_default()
        } else {
            String::new()
        };
        let id = ["AXIdentifier", "AXDOMIdentifier"]
            .into_iter()
            .filter_map(|name| string_attribute(&self.element, name))
            .find(|id| !id.trim().is_empty())
            .unwrap_or_default();
        let label = self.labels().first().cloned().unwrap_or_default();
        let who = if site.is_empty() {
            NSRunningApplication::runningApplicationWithProcessIdentifier(self.pid)
                .and_then(|a| a.localizedName())
                .map(|name| name.to_string())
                .unwrap_or_else(|| app.clone())
        } else {
            site.clone()
        };
        Place::new(app, site, id, label, &who)
    }

    /// The field's text, empty while it only shows its placeholder. Web
    /// composers draw the placeholder as text: ChatGPT's empty box in Zen
    /// reads "Ask ChatGPT\n", with no AXPlaceholderValue, the same words in
    /// AXDescription. Checking it underlined the placeholder.
    fn value(&self) -> Option<String> {
        let value = string_attribute(&self.element, "AXValue")?;
        let shown = value.trim();
        if shown.is_empty() {
            return Some(value);
        }
        let placeholder = self.labels().iter().any(|label| label == shown);
        let text = if placeholder {
            String::new()
        } else if web(&self.element) {
            self.with_lines(value)
        } else {
            value
        };
        if self.hidden.borrow().0 != text {
            let map = align(&self.element, &text);
            *self.hidden.borrow_mut() = (text.clone(), map, true);
            let reliable = self.offsets_hold(&text);
            self.hidden.borrow_mut().2 = reliable;
        }
        Some(text)
    }

    /// The part of the field actually on screen. A long text view is as tall
    /// as its whole document (TextEdit reported 2977pt for a window a third of
    /// that), so its own frame cannot clip anything; the scroll area around it
    /// is the visible part. Walks up at most three parents to find one.
    fn visible_frame(&self) -> Option<Rect> {
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
                return Some(intersect(own, area).into());
            }
            current = parent;
        }
        Some(own.into())
    }

    fn bounds(&self, start: usize, len: usize) -> Option<Rect> {
        let (location, length) = self.utf16(start, len);
        self.bounds_utf16(location, length).map(Rect::from)
    }

    fn select(&self, start: usize, len: usize) -> bool {
        let (location, length) = self.utf16(start, len);
        self.select_utf16(location, length)
    }

    fn replace(&self, start: usize, len: usize, text: &str, before: &str, expected: &str) -> Write {
        let (location, length) = utf16_range(before, start, start + len);
        self.replace_utf16(location, length, text, before, expected)
    }
}

/// The text of each line a web field shows, in order: each group that holds
/// text of its own (a paragraph, a list item), with its list marker.
fn blocks_of(field: &AXUIElement) -> Vec<String> {
    const MOST_NODES: usize = 400;
    let mut blocks = Vec::new();
    let mut visited = 0;
    let mut stack: Vec<CFRetained<AXUIElement>> = children_of(field).into_iter().rev().collect();
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > MOST_NODES {
            return Vec::new();
        }
        let children = children_of(&node);
        let texts = |n: &AXUIElement| {
            matches!(
                string_attribute(n, "AXRole").as_deref(),
                Some("AXStaticText" | "AXListMarker")
            )
        };
        if children.iter().any(|c| texts(c)) {
            // A line: its text is every piece of text inside it.
            let mut line = String::new();
            let mut inner: Vec<CFRetained<AXUIElement>> = children.into_iter().rev().collect();
            while let Some(piece) = inner.pop() {
                visited += 1;
                if texts(&piece) {
                    line.push_str(&string_attribute(&piece, "AXValue").unwrap_or_default());
                } else {
                    inner.extend(children_of(&piece).into_iter().rev());
                }
            }
            blocks.push(line);
        } else {
            stack.extend(children.into_iter().rev());
        }
    }
    blocks
}

/// `value` with a line break after each of `blocks` but the last, where it
/// has none. Unchanged unless every block is found, in order.
fn add_line_breaks(value: &str, blocks: &[String]) -> String {
    let blocks: Vec<&String> = blocks.iter().filter(|b| !b.trim().is_empty()).collect();
    if blocks.len() < 2 {
        return value.to_string();
    }
    let mut breaks = Vec::new();
    let mut cursor = 0;
    for (i, block) in blocks.iter().enumerate() {
        let Some(at) = value[cursor..].find(block.as_str()) else {
            return value.to_string();
        };
        let end = cursor + at + block.len();
        if i + 1 < blocks.len() && !value[end..].starts_with('\n') {
            breaks.push(end);
        }
        cursor = end;
    }
    let mut out = String::with_capacity(value.len() + breaks.len());
    let mut last = 0;
    for at in breaks {
        out.push_str(&value[last..at]);
        out.push('\n');
        last = at;
    }
    out.push_str(&value[last..]);
    out
}

/// How the app counts `text`; see [`Field::hidden`]. Asked of
/// `AXStringForRange`, which counts as the app's selection and bounds do,
/// and aligned with the value unit by unit: a unit the app has and the
/// value lacks (Zen's object character, a list's bullet) is skipped in the
/// app's count, and a line break the value has and the app lacks (Chrome
/// between paragraphs) maps to nothing. None, so offsets pass through
/// untouched, when the two agree, when they differ in any other way, or
/// when the app cannot say: [`Field::select`] still finds the text before
/// writing.
fn align(element: &AXUIElement, text: &str) -> Option<OffsetMap> {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.is_empty() {
        return None;
    }
    let newline = u16::from(b'\n');
    let limit = 16 + units.len() / 5;
    // Chrome answers nothing for a range that runs past its own count,
    // which is shorter than the value: then the longest it will answer.
    let app = match string_for_range(element, 0, units.len()) {
        Some(app) => app,
        None => {
            let (mut lo, mut hi) = (0, units.len());
            while lo < hi {
                let mid = (lo + hi).div_ceil(2);
                if string_for_range(element, 0, mid).is_some() {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            string_for_range(element, 0, lo)?
        }
    };
    let mut app: Vec<u16> = app.encode_utf16().collect();
    if app == units {
        return None;
    }
    // Whichever is shorter lacks what the other has: Zen's chip is a unit
    // the app counts and the value lacks; its list bullets the other way.
    let fewer = app.len() < units.len();
    // The app's string is as long as the value plus its extras, which are
    // only known once found: ask again, longer, until the value is covered.
    loop {
        let mut map = Vec::with_capacity(units.len());
        let (mut i, mut j, mut extras) = (0, 0, 0);
        while j < units.len() {
            if i < app.len() && app[i] == units[j] {
                map.push(Some(i));
                i += 1;
                j += 1;
            } else if units[j] == newline || fewer {
                // The value has a unit the app does not count: a line break
                // Chrome or Zen leaves out, a bullet Zen shows and skips.
                map.push(None);
                j += 1;
                extras += 1;
            } else if i < app.len() {
                i += 1;
                extras += 1;
            } else {
                break;
            }
            if extras > limit {
                return None;
            }
        }
        if j == units.len() {
            return Some(map);
        }
        let longer = app.len() + (units.len() - j);
        app = string_for_range(element, 0, longer)?
            .encode_utf16()
            .collect();
        if app.len() < longer {
            return None;
        }
    }
}

fn string_for_range(element: &AXUIElement, location: usize, length: usize) -> Option<String> {
    let range = range_value(location, length)?;
    let name = CFString::from_static_str("AXStringForRange");
    let mut value: *const CFType = std::ptr::null();
    let error = unsafe {
        element.copy_parameterized_attribute_value(&name, &range, NonNull::from(&mut value))
    };
    if error != AXError::Success || value.is_null() {
        return None;
    }
    let value = unsafe { CFRetained::from_raw(NonNull::new_unchecked(value as *mut CFType)) };
    value.downcast_ref::<CFString>().map(|s| s.to_string())
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

/// A raw description of whatever has focus, for `squiggle doctor --raw`: the
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

/// The text field of app `pid` whose `AXIdentifier` (an AppKit view's
/// identifier) or `AXDOMIdentifier` (a web element's id) is `id`. For
/// `squiggle lab`, which finds its test fields this way rather than by focus,
/// so the test window never has to come forward.
///
/// With `window`, only windows whose title contains it are searched: a
/// browser holds every open tab's page, and other pages use the same ids.
pub fn find(pid: i32, window: Option<&str>, id: &str) -> Option<Field> {
    let app = unsafe { AXUIElement::new_application(pid) };
    unsafe { app.set_messaging_timeout(2.0) };
    let mut queue = match window {
        None => vec![(app, 0)],
        Some(title) => children_of(&app)
            .into_iter()
            .filter(|w| string_attribute(w, "AXTitle").is_some_and(|t| t.contains(title)))
            .map(|w| (w, 1))
            .collect(),
    };
    while let Some((element, depth)) = queue.pop() {
        let named = ["AXIdentifier", "AXDOMIdentifier"]
            .into_iter()
            .any(|name| string_attribute(&element, name).as_deref() == Some(id));
        if named {
            unsafe { element.set_messaging_timeout(2.0) };
            return Some(Field::new(element));
        }
        if depth < 80 {
            queue.extend(children_of(&element).into_iter().map(|c| (c, depth + 1)));
        }
    }
    None
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
