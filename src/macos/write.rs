//! Writing accepted fixes into the field: through Accessibility when the
//! app honours it, as a paste over a confirmed selection when it does not.
//! Shared by the app and by `gramly lab`, which runs this same code against
//! real text fields to audit it.
//!
//! When in doubt nothing is written: every refusal says why, and the caller
//! logs it.

use crate::macos::ax::{Field, Write};
use crate::macos::paste;
use gramly_core::{Fix, apply};

/// Why a fix was not written.
pub type Refusal = &'static str;

/// One fix. Returns the text the field should now hold.
pub fn one(field: &Field, text: &str, fix: &Fix) -> Result<String, Refusal> {
    unchanged(field, text)?;
    let expected = apply(text, std::slice::from_ref(fix));
    let (location, length) = fix.utf16(text);
    match field.replace(location, length, &fix.replacement, text, &expected) {
        Write::Done => {}
        Write::Ignored => paste_over(field, location, length, text, &fix.replacement)?,
        Write::Unsafe => return Err("the write could not be confirmed"),
    }
    Ok(expected)
}

/// Several fixes at once, back to front so each write leaves the offsets of
/// the rest valid. If the app refuses direct writes, the span they cover is
/// pasted once, corrected - a paste per fix would race the clipboard.
/// Returns the text the field should now hold.
pub fn many(field: &Field, text: &str, fixes: &[Fix]) -> Result<String, Refusal> {
    let mut fixes = fixes.to_vec();
    fixes.sort_by_key(|f| f.start);
    let (Some(lo), Some(hi)) = (
        fixes.iter().map(|f| f.start).min(),
        fixes.iter().map(|f| f.end).max(),
    ) else {
        return Ok(text.to_string());
    };
    unchanged(field, text)?;
    let corrected = apply(text, &fixes);
    let mut current = text.to_string();
    let mut direct = true;
    for fix in fixes.iter().rev() {
        let next = apply(&current, std::slice::from_ref(fix));
        let (location, length) = fix.utf16(&current);
        match field.replace(location, length, &fix.replacement, &current, &next) {
            Write::Done => current = next,
            Write::Ignored => {
                direct = false;
                break;
            }
            Write::Unsafe => return Err("a write could not be confirmed"),
        }
    }
    if !direct {
        // Every change lies inside [lo, hi), so outside it `current`, `text`
        // and `corrected` agree and only the span's end moves.
        let len = |s: &str| s.chars().count() as isize;
        let current_hi = (hi as isize + len(&current) - len(text)) as usize;
        let corrected_hi = (hi as isize + len(&corrected) - len(text)) as usize;
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
        // Pasted across a line, a web list re-split its items around the
        // paste and left a stray space (`gramly lab`, "Web list"). The app
        // groups by sentence, which never crosses a line, so this only
        // guards an app that reports its lines some other way.
        if span.original(&current).contains('\n') || replacement.contains('\n') {
            return Err("the fixes span more than one line");
        }
        paste_over(field, location, length, &current, &replacement)?;
    }
    Ok(corrected)
}

/// Whether the field still holds `text`, the text the fixes were found in.
/// Read now rather than trusted from the last tick: offsets into text that
/// has moved on select the wrong words.
fn unchanged(field: &Field, text: &str) -> Result<(), Refusal> {
    if field.value().as_deref() == Some(text) {
        Ok(())
    } else {
        Err("the text changed before the fix was applied")
    }
}

/// The paste fallback, only over a confirmed selection of text that is
/// still what it was: a paste replaces whatever is selected, and over the
/// wrong selection it replaced a whole paragraph with one word.
fn paste_over(
    field: &Field,
    location: usize,
    length: usize,
    before: &str,
    replacement: &str,
) -> Result<(), Refusal> {
    if !field.select(location, length) {
        return Err("the app did not confirm the selection");
    }
    // A write that was slow rather than ignored may land while selecting.
    if field.value().as_deref() != Some(before) {
        return Err("the text changed while selecting");
    }
    // A removal is a Delete over the selection, not an empty paste: Slack
    // ignored the empty paste and the comma stayed (log: "the field holds 68
    // chars, expected 67").
    if replacement.is_empty() {
        paste::delete();
    } else {
        paste::paste(replacement);
    }
    Ok(())
}
