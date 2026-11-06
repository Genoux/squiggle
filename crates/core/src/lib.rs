//! Everything Squiggle knows about text, and nothing about screens.
//!
//! The platforms (`src/macos`, later `src/linux`) read the focused field, ask
//! this crate what to fix, and draw the answer. Keeping it free of any
//! windowing code is what lets CI test it on Linux and macOS alike.

pub mod ai;
pub mod config;
pub mod diff;
pub mod fix;
pub mod improve;
pub mod lang;
pub mod local;
pub mod off;
pub mod protect;
pub mod router;
pub mod worker;

pub use config::Config;
pub use fix::{Fix, Kind, Source, apply, merge, rebase, sentence};
pub use local::Local;
