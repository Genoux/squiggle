//! Gramly's font on a machine that does not have it. The bytes the binary
//! carries are written once under the runtime directory and handed to
//! fontconfig as application fonts, which only this process sees: nothing is
//! installed, and an uninstall leaves nothing behind. Registered before GTK
//! starts, so the font map pango builds already holds them. Without them the
//! stylesheet's next family, the system sans, is drawn.

use crate::surface::theme::FONTS;
use std::ffi::{CString, c_int, c_uchar, c_void};
use std::path::{Path, PathBuf};

#[link(name = "fontconfig")]
unsafe extern "C" {
    /// `FcConfigAppFontAddFile`; a null config is the current one.
    fn FcConfigAppFontAddFile(config: *mut c_void, file: *const c_uchar) -> c_int;
}

/// The runtime directory only: it is the user's alone (0700). A shared
/// `/tmp` would let another user plant a font this process loads.
fn folder() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("gramly/fonts"))
}

/// Written beside and renamed into place, so a second instance starting at
/// the same moment never reads half a font.
fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|found| found == bytes) {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let partial = path.with_extension(format!("{}.part", std::process::id()));
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, path)
}

fn add(path: &Path) -> bool {
    let Ok(file) = CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    // SAFETY: `file` is a NUL-terminated path that outlives the call, and
    // fontconfig copies it.
    unsafe { FcConfigAppFontAddFile(std::ptr::null_mut(), file.as_ptr().cast()) != 0 }
}

pub fn register() {
    let Some(folder) = folder() else {
        note!("no XDG_RUNTIME_DIR: Gramly's font not registered");
        return;
    };
    for (weight, bytes) in FONTS {
        let path = folder.join(format!("atkinson-{}.ttf", weight.css()));
        if let Err(why) = write(&path, bytes) {
            note!("font not written to {}: {why}", path.display());
        } else if !add(&path) {
            note!("fontconfig did not take {}", path.display());
        }
    }
}
