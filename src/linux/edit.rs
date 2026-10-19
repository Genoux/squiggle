//! The decisions behind writing into a field, apart from the calls that
//! need an app on the other end.

use atspi_proxies::common::Role;
use std::time::Duration;

/// How long a confirmed selection must hold before it is pasted over.
pub const STEADY: Duration = Duration::from_millis(30);
/// Parents asked after before giving up on finding a web document.
pub const ANCESTORS: usize = 30;

/// Whether an app's toolkit puts a web engine behind its text fields.
/// Electron reports Chromium.
pub fn web_toolkit(name: &str) -> bool {
    ["Gecko", "Chromium"]
        .iter()
        .any(|engine| engine.eq_ignore_ascii_case(name))
}

/// Whether a field is part of a web page. A Gecko or Chromium app is web
/// throughout: Gecko's `EditableText` lands late and at the caret, and
/// Firefox's URL bar has no `DocumentWeb` above it. Anything else is web when
/// it offers no `EditableText` or sits under a `DocumentWeb`.
pub fn is_web(toolkit: &str, editable: bool, ancestors: &[Role]) -> bool {
    web_toolkit(toolkit) || !editable || ancestors.contains(&Role::DocumentWeb)
}

/// Whether a native write stopped halfway: the delete took and the insert did
/// not, so the field reads `before` less the range. Left so, the fix was
/// half a correction and the paste that follows an ignored write would land
/// on text that is not what it was found in.
pub fn half_written(
    before: &str,
    after: Option<&str>,
    expected: &str,
    start: usize,
    len: usize,
) -> bool {
    let Some(after) = after else {
        return false;
    };
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let chars: Vec<char> = before.chars().collect();
    if len == 0 || after == expected || chars.get(start..end).is_none() {
        return false;
    }
    let without: String = chars[..start].iter().chain(&chars[end..]).collect();
    after == without
}

/// The `len` characters of `text` from `start`.
pub fn span(text: &str, start: usize, len: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    Some(chars.get(start..start.checked_add(len)?)?.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_without_editable_text_is_web() {
        assert!(is_web("GTK", false, &[Role::Frame]));
    }

    #[test]
    fn an_editable_field_under_a_web_document_is_web() {
        assert!(is_web(
            "GTK",
            true,
            &[Role::Section, Role::DocumentWeb, Role::Frame]
        ));
    }

    #[test]
    fn a_native_editable_field_is_not_web() {
        assert!(!is_web(
            "GTK",
            true,
            &[Role::Panel, Role::Frame, Role::Application]
        ));
        assert!(!is_web("GTK", true, &[]));
    }

    #[test]
    fn a_gecko_or_chromium_app_is_web_whatever_is_above_its_field() {
        assert!(is_web("Gecko", true, &[Role::Frame, Role::Application]));
        assert!(is_web("Gecko", true, &[]));
        assert!(is_web("Chromium", true, &[Role::Panel]));
        assert!(is_web("chromium", true, &[]));
    }

    #[test]
    fn a_range_deleted_but_never_replaced_is_half_written() {
        assert!(half_written("a teh cat", Some("a  cat"), "a the cat", 2, 3));
    }

    #[test]
    fn a_write_that_finished_or_never_started_is_not_half_written() {
        assert!(!half_written(
            "a teh cat",
            Some("a the cat"),
            "a the cat",
            2,
            3
        ));
        assert!(!half_written(
            "a teh cat",
            Some("a teh cat"),
            "a the cat",
            2,
            3
        ));
        assert!(!half_written("a teh cat", None, "a the cat", 2, 3));
    }

    #[test]
    fn something_else_changed_is_not_half_written() {
        assert!(!half_written(
            "a teh cat",
            Some("a  dog"),
            "a the cat",
            2,
            3
        ));
    }

    #[test]
    fn deleting_the_whole_replacement_is_not_half_written() {
        assert!(!half_written("a teh cat", Some("a  cat"), "a  cat", 2, 3));
    }

    #[test]
    fn an_insertion_deletes_nothing() {
        assert!(!half_written("a cat", Some("a cat"), "a big cat", 2, 0));
    }

    #[test]
    fn a_range_outside_the_text_is_not_half_written() {
        assert!(!half_written("abc", Some("ab"), "abd", 2, 5));
        assert!(!half_written("abc", Some("ab"), "abd", usize::MAX, 2));
    }

    #[test]
    fn a_span_counts_characters() {
        assert_eq!(span("Hello 😀 wrold", 8, 5).as_deref(), Some("wrold"));
        assert_eq!(span("abc", 1, 0).as_deref(), Some(""));
        assert_eq!(span("abc", 2, 2), None);
        assert_eq!(span("abc", usize::MAX, 2), None);
    }
}
