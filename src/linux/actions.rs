//! What the menu items that touch files and the clipboard do.

use crate::surface::recent_log;
use gramly_core::config::dictionary_path;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use wl_clipboard_rs::copy::{self, MimeType, Options, Source};

/// Where the systemd unit appends stderr: `%S/gramly/gramly.log`, which
/// systemd resolves the same way for a user manager.
pub fn log_path() -> PathBuf {
    log_path_in(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

fn log_path_in(state_home: Option<OsString>, home: Option<OsString>) -> PathBuf {
    let state = state_home
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home.unwrap_or_default()).join(".local/state"));
    state.join("gramly/gramly.log")
}

pub fn copy_recent_log() {
    let path = log_path();
    let copied = std::fs::read_to_string(&path)
        .map_err(|why| format!("could not read {}: {why}", path.display()))
        .and_then(|log| {
            let source = Source::Bytes(recent_log(&log).into_bytes().into());
            copy::copy(Options::new(), source, MimeType::Text).map_err(|why| why.to_string())
        });
    match copied {
        Ok(()) => note!("copied the end of the log to the clipboard"),
        Err(why) => note!("recent log not copied: {why}"),
    }
}

// ponytail: the dictionary opens in whatever edits text files, where macOS
// has a list window. Upgrade path: a GTK list window with add and remove,
// calling `Engine::recheck` after a change, as the macOS one does.
pub fn open_dictionary() {
    let path = dictionary_path();
    let ready = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map(drop)
        });
    if let Err(why) = ready {
        note!("dictionary not created at {}: {why}", path.display());
        return;
    }
    if let Err(why) = Command::new("xdg-open").arg(&path).spawn() {
        note!("xdg-open failed for {}: {why}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_follows_xdg_state_home() {
        assert_eq!(
            log_path_in(Some("/state".into()), Some("/home/u".into())),
            PathBuf::from("/state/gramly/gramly.log")
        );
    }

    #[test]
    fn the_log_defaults_under_local_state() {
        for empty in [None, Some(OsString::new())] {
            assert_eq!(
                log_path_in(empty, Some("/home/u".into())),
                PathBuf::from("/home/u/.local/state/gramly/gramly.log")
            );
        }
    }
}
