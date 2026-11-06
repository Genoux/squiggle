//! The paste fallback's clipboard half: stage the text, and after the
//! keystroke put the clipboard back.
//!
//! Chromium and Electron offer no `EditableText`, so for them a fix is a paste
//! over a selection. The user's clipboard comes back afterwards, every type
//! it offered, as on macOS: a paste that eats a copied image is a bug report.
//!
//! Staging runs on the caller's thread, so a refusal is known before anything
//! is selected, but every read of the clipboard is a pipe to whoever owns it
//! and is bounded by [`PIPE_TIMEOUT`]. Only the wait before restoring runs on
//! a thread of its own.

use crate::surface::write::Refusal;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use wl_clipboard_rs::copy::{self, MimeSource, MimeType, Options, Source};
use wl_clipboard_rs::paste::{self, ClipboardType, Seat};

/// How long the target app gets to read the clipboard before it is
/// restored. Measured on macOS against Chrome and Slack, which read on the
/// next turn of their loop; restoring sooner pasted the old clipboard.
const RESTORE_AFTER: Duration = Duration::from_millis(250);
/// An owner that never closes its pipe must not hold up the fix.
const PIPE_TIMEOUT: Duration = Duration::from_millis(500);
const PLAIN_TEXT: &str = "text/plain;charset=utf-8";
/// Everything `wl-clipboard-rs` offers for text it is given with `sensitive`
/// set: any other type on the clipboard means someone has copied since.
const OFFERED: &[&str] = &[
    PLAIN_TEXT,
    "text/plain",
    "STRING",
    "UTF8_STRING",
    "TEXT",
    "x-kde-passwordManagerHint",
];

/// What the clipboard held: each type it offered, with its bytes.
type Saved = Vec<(String, Vec<u8>)>;

/// The user's clipboard, kept while Squiggle's text is on it, and that text.
struct Staged {
    saved: Saved,
    offered: String,
}

static STAGED: Mutex<Option<Staged>> = Mutex::new(None);
/// One paste at a time, from staging until the clipboard is back: a second
/// would save the first's temporary text as the user's clipboard.
static BUSY: AtomicBool = AtomicBool::new(false);

fn staged() -> MutexGuard<'static, Option<Staged>> {
    STAGED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Saves the clipboard and puts `text` on it. Refused when the clipboard
/// cannot be saved whole, because it could not be put back.
pub fn stage(text: &str) -> Result<(), Refusal> {
    if BUSY.swap(true, Ordering::AcqRel) {
        return Err("a paste is already in flight");
    }
    match prepare(text) {
        Ok(ready) => {
            *staged() = Some(ready);
            Ok(())
        }
        Err(refusal) => {
            BUSY.store(false, Ordering::Release);
            Err(refusal)
        }
    }
}

fn prepare(text: &str) -> Result<Staged, Refusal> {
    let saved = within(read).map_err(|why| {
        note!("clipboard not saved: {why}");
        "the clipboard could not be saved"
    })?;
    if let Err(why) = offer(text) {
        note!("fix not pasted: {why}");
        // The offer may have landed before it was refused.
        restore(saved);
        return Err("the clipboard did not take the text");
    }
    Ok(Staged {
        saved,
        offered: text.to_owned(),
    })
}

/// Puts the clipboard back once the target app has had time to read it.
pub fn restore_after_paste() {
    give_back_in(RESTORE_AFTER);
}

/// Puts the clipboard back at once: nothing was pasted.
pub fn restore_now() {
    give_back_in(Duration::ZERO);
}

fn give_back_in(delay: Duration) {
    let Some(ready) = staged().take() else {
        return;
    };
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        give_back(ready);
        BUSY.store(false, Ordering::Release);
    });
}

/// Restores only a clipboard that is still Squiggle's offer. One the user has
/// copied to since is theirs and stays.
fn give_back(ready: Staged) {
    match within(current) {
        Ok((types, text)) if still_ours(&types, text.as_deref(), &ready.offered) => {
            restore(ready.saved);
        }
        Ok(_) => note!("clipboard changed since the paste, left as it is"),
        Err(why) => note!("clipboard not read before restoring, left as it is: {why}"),
    }
}

/// Whether the clipboard is the offer Squiggle made: only the types it offers,
/// holding its text.
fn still_ours(types: &[String], text: Option<&[u8]>, offered: &str) -> bool {
    !types.is_empty()
        && types.iter().all(|kind| OFFERED.contains(&kind.as_str()))
        && text == Some(offered.as_bytes())
}

/// Runs `work` on a thread and waits for it no longer than [`PIPE_TIMEOUT`].
fn within<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (answer, done) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = answer.send(work());
    });
    done.recv_timeout(PIPE_TIMEOUT)
        .map_err(|_| "the owner did not answer in time".to_string())?
}

fn mime_types() -> Result<Vec<String>, String> {
    match paste::get_mime_types_ordered(ClipboardType::Regular, Seat::Unspecified) {
        Ok(types) => Ok(types),
        Err(paste::Error::ClipboardEmpty) => Ok(Vec::new()),
        Err(why) => Err(why.to_string()),
    }
}

/// Logs the slow parts: a save that runs out of time is given up on, and
/// this thread's note is the only account of what held it.
fn read() -> Result<Saved, String> {
    let started = Instant::now();
    let types = mime_types()?;
    slow("listing the types", started);
    saved_all(types, |kind| {
        let asked = Instant::now();
        let bytes = contents(kind);
        slow(kind, asked);
        bytes
    })
}

fn slow(what: &str, since: Instant) {
    let took = since.elapsed();
    if took >= Duration::from_millis(100) {
        note!("clipboard: {what} took {}ms", took.as_millis());
    }
}

/// Every type with its bytes, or why one could not be read: a clipboard
/// restored without a type it had is not the one the user had.
fn saved_all(
    types: Vec<String>,
    mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Saved, String> {
    types
        .into_iter()
        .map(|kind| {
            let bytes = read(&kind).map_err(|why| format!("{kind}: {why}"))?;
            Ok((kind, bytes))
        })
        .collect()
}

fn current() -> Result<(Vec<String>, Option<Vec<u8>>), String> {
    Ok((mime_types()?, contents(PLAIN_TEXT).ok()))
}

fn contents(kind: &str) -> Result<Vec<u8>, String> {
    let (mut pipe, _) = paste::get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        paste::MimeType::Specific(kind),
    )
    .map_err(|why| why.to_string())?;
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)
        .map_err(|why| why.to_string())?;
    Ok(bytes)
}

/// Offers `text` as plain text, hinted so history managers skip it, and
/// reads it back: that read only answers once the compositor has the offer,
/// and a Ctrl+V before then would paste what was there. Only the read is
/// bounded, since it reaches whoever owns the clipboard by then. The offer
/// waits on the compositor alone, and an offer given up on could land after
/// the restore and replace the user's clipboard.
fn offer(text: &str) -> Result<(), String> {
    let mut options = Options::new();
    options.sensitive(true);
    copy::copy(
        options,
        Source::Bytes(text.as_bytes().into()),
        MimeType::Text,
    )
    .map_err(|why| why.to_string())?;
    if within(|| contents(PLAIN_TEXT))? == text.as_bytes() {
        Ok(())
    } else {
        Err("the clipboard did not take the text".into())
    }
}

fn restore(saved: Saved) {
    let sources = restore_sources(saved);
    let restored = if sources.is_empty() {
        copy::clear(copy::ClipboardType::Regular, copy::Seat::All)
    } else {
        let mut options = Options::new();
        options.omit_additional_text_mime_types(true);
        copy::copy_multi(options, sources)
    };
    if let Err(why) = restored {
        note!("clipboard not restored: {why}");
    }
}

/// Every saved type, offered exactly as it was.
fn restore_sources(saved: Saved) -> Vec<MimeSource> {
    saved
        .into_iter()
        .map(|(kind, bytes)| MimeSource {
            source: Source::Bytes(bytes.into()),
            mime_type: MimeType::Specific(kind),
        })
        .collect()
}

#[cfg(test)]
mod tests {

    use super::*;

    fn kinds(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn every_saved_type_comes_back_with_its_bytes() {
        let saved = vec![
            ("text/plain".to_string(), b"hello".to_vec()),
            ("image/png".to_string(), vec![0x89, b'P', b'N', b'G']),
        ];
        assert_eq!(
            restore_sources(saved),
            [
                MimeSource {
                    source: Source::Bytes(b"hello"[..].into()),
                    mime_type: MimeType::Specific("text/plain".into()),
                },
                MimeSource {
                    source: Source::Bytes([0x89, b'P', b'N', b'G'][..].into()),
                    mime_type: MimeType::Specific("image/png".into()),
                },
            ]
        );
    }

    #[test]
    fn an_empty_clipboard_restores_nothing() {
        assert_eq!(restore_sources(Vec::new()), []);
    }

    #[test]
    fn a_clipboard_is_saved_whole_or_not_at_all() {
        let types = kinds(&["text/plain", "image/png"]);
        let whole = saved_all(types.clone(), |kind| Ok(kind.as_bytes().to_vec()));
        assert_eq!(whole.map(|saved| saved.len()), Ok(2));

        let partial = saved_all(types, |kind| match kind {
            "image/png" => Err("pipe closed".into()),
            _ => Ok(Vec::new()),
        });
        assert_eq!(partial, Err("image/png: pipe closed".to_string()));
    }

    #[test]
    fn an_empty_clipboard_is_saved_as_nothing() {
        assert_eq!(saved_all(Vec::new(), |_| unreachable!()), Ok(Vec::new()));
    }

    #[test]
    fn the_offer_is_ours_while_it_is_the_only_thing_there() {
        let types = kinds(&[
            "text/plain;charset=utf-8",
            "UTF8_STRING",
            "x-kde-passwordManagerHint",
        ]);
        assert!(still_ours(&types, Some(b"the"), "the"));
    }

    #[test]
    fn a_new_copy_is_not_ours() {
        let html = kinds(&["text/plain;charset=utf-8", "text/html"]);
        assert!(!still_ours(&html, Some(b"the"), "the"));
        let other = kinds(&["text/plain;charset=utf-8", "UTF8_STRING"]);
        assert!(!still_ours(&other, Some(b"something else"), "the"));
        assert!(!still_ours(&other, None, "the"));
    }

    #[test]
    fn an_empty_clipboard_is_not_ours() {
        assert!(!still_ours(&[], None, "the"));
    }
}
