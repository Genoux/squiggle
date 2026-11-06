//! Squiggle: grammar suggestions in whatever text field has focus.
//!
//! `squiggle` runs the app. `squiggle check` reads text on stdin and prints what
//! would be underlined - the same engine, without a screen, for debugging a
//! suggestion and for scripts.

/// A line in the log, with the time: "it happened again" can only be
/// matched to a log line that says when. Defined before the modules so
/// every one of them sees it.
macro_rules! note {
    ($($arg:tt)*) => {
        $crate::note_line(&format!($($arg)*))
    };
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod surface;

use anyhow::{Result, bail};
use squiggle_core::{Config, Local, ai, config, improve, protect};
use std::io::Read;

fn main() -> Result<()> {
    config::adopt_old_dir();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("run") => run(),
        Some("check") => check(
            args.iter().any(|a| a == "--local"),
            args.iter().any(|a| a == "--improve"),
        ),
        #[cfg(target_os = "macos")]
        Some("doctor") => macos::doctor(),
        #[cfg(target_os = "linux")]
        Some("doctor") => linux::doctor(),
        #[cfg(target_os = "macos")]
        Some("lab") => macos::lab(&args[1..]),
        #[cfg(target_os = "macos")]
        Some("lab-host") => macos::lab_host(),
        #[cfg(target_os = "macos")]
        Some("preview") => macos::preview(&args[1..]),
        Some("--version" | "-V") => {
            println!("squiggle {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("help" | "--help" | "-h") => {
            println!(
                "squiggle            run\n\
                 squiggle check      print suggestions for text on stdin\n\
                 squiggle check --local   the on-device pass only, nothing sent\n\
                 squiggle check --improve   the clearer wording the app proposes\n\
                 squiggle doctor     what Squiggle can see in the focused field\n\
                 squiggle lab        audit accepting fixes in real text fields, off screen (macOS)\n\
                 squiggle preview [dir]   draw every card and the dictionary to PNG (macOS)\n\
                 squiggle --version"
            );
            Ok(())
        }
        Some(other) => bail!("unknown command {other:?} (try `squiggle help`)"),
    }
}

fn run() -> Result<()> {
    #[cfg(target_os = "macos")]
    return macos::run();
    #[cfg(target_os = "linux")]
    return linux::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    bail!("this platform has no surface yet; `squiggle check` works everywhere")
}

/// The model's fixes for `text`, asked a paragraph at a time as the app
/// asks: sent whole, Coda's text came back without its blank lines, the
/// lines no longer matched, and nothing was shown.
fn by_paragraph(
    text: &str,
    ask: impl Fn(&str) -> Result<Vec<squiggle_core::Fix>>,
) -> Result<Vec<squiggle_core::Fix>> {
    let mut fixes = Vec::new();
    for (offset, paragraph) in squiggle_core::worker::asked(text) {
        fixes.extend(ask(&paragraph)?.into_iter().map(|mut fix| {
            fix.start += offset;
            fix.end += offset;
            fix
        }));
    }
    Ok(fixes)
}

fn check(local_only: bool, improving: bool) -> Result<()> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let config = Config::load()?;
    if improving {
        let Some(key) = config.openrouter_key.as_deref() else {
            bail!("improvements need the model, and there is no OpenRouter key");
        };
        let fixes = protect::keep(&text, by_paragraph(&text, |p| improve::check(key, p))?);
        for fix in protect::keep_known(&text, fixes, &config::dictionary()) {
            println!(
                "{:>4}..{:<4} {:?} -> {:?}",
                fix.start,
                fix.end,
                fix.original(&text),
                fix.replacement
            );
        }
        return Ok(());
    }
    let mut local = Local::new(config.harper_dialect());
    local.set_known_words(config::dictionary());
    // What the app draws once the model has answered: the model's fixes
    // alone, or Harper's when the model is off.
    let fixes = match (&config.openrouter_key, config.ai && !local_only) {
        (Some(key), true) => by_paragraph(&text, |p| ai::check(key, p, &config.dialect))?,
        _ => local.check(&text),
    };
    let fixes = protect::keep(&text, fixes);
    for fix in protect::keep_known(&text, fixes, &config::dictionary()) {
        println!(
            "{:>4}..{:<4} {:?} {:?} -> {:?}  {}",
            fix.start,
            fix.end,
            fix.kind,
            fix.original(&text),
            fix.replacement,
            fix.message
        );
    }
    Ok(())
}

pub(crate) fn note_line(message: &str) {
    eprintln!("{} squiggle: {message}", local_time());
}

fn local_time_at(at: std::time::SystemTime) -> libc::tm {
    let seconds = at
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as libc::time_t;
    let mut broken_down: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&seconds, &mut broken_down) };
    broken_down
}

/// "yyyy-MM-dd HH:mm:ss.SSS" in the machine's time zone.
fn local_time() -> String {
    let now = std::time::SystemTime::now();
    let broken_down = local_time_at(now);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        broken_down.tm_year + 1900,
        broken_down.tm_mon + 1,
        broken_down.tm_mday,
        broken_down.tm_hour,
        broken_down.tm_min,
        broken_down.tm_sec,
        now.duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_millis()
    )
}

/// "HH:MM" in the machine's time zone.
#[cfg(target_os = "linux")]
fn clock_time(at: std::time::SystemTime) -> String {
    let broken_down = local_time_at(at);
    format!("{:02}:{:02}", broken_down.tm_hour, broken_down.tm_min)
}

#[cfg(test)]
mod tests {
    use super::local_time;
    #[cfg(target_os = "linux")]
    use super::{clock_time, local_time_at};
    #[cfg(target_os = "linux")]
    use std::time::{Duration, SystemTime};

    #[test]
    fn log_time_has_the_shape_the_log_always_had() {
        let shape: String = local_time()
            .chars()
            .map(|c| if c.is_ascii_digit() { '9' } else { c })
            .collect();
        assert_eq!(shape, "9999-99-99 99:99:99.999");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn clock_time_is_hours_and_minutes() {
        let shape: String = clock_time(SystemTime::now())
            .chars()
            .map(|c| if c.is_ascii_digit() { '9' } else { c })
            .collect();
        assert_eq!(shape, "99:99");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn clock_time_follows_the_time_it_is_given() {
        let now = SystemTime::now();
        let later = now + Duration::from_secs(60);
        let minute = |at| local_time_at(at).tm_min;
        assert_eq!((minute(now) + 1) % 60, minute(later));
    }
}
