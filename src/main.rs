//! Gramly: grammar suggestions in whatever text field has focus.
//!
//! `gramly` runs the app. `gramly check` reads text on stdin and prints what
//! would be underlined - the same engine, without a screen, for debugging a
//! suggestion and for scripts.

#[cfg(target_os = "macos")]
mod macos;

use anyhow::{Result, bail};
use gramly_core::{Config, Local, ai, config, improve, protect};
use std::io::Read;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("run") => run(),
        Some("check") => check(
            args.iter().any(|a| a == "--local"),
            args.iter().any(|a| a == "--improve"),
        ),
        #[cfg(target_os = "macos")]
        Some("doctor") => macos::doctor(),
        #[cfg(target_os = "macos")]
        Some("lab") => macos::lab(&args[1..]),
        #[cfg(target_os = "macos")]
        Some("lab-host") => macos::lab_host(),
        #[cfg(target_os = "macos")]
        Some("preview") => macos::preview(&args[1..]),
        Some("--version" | "-V") => {
            println!("gramly {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("help" | "--help" | "-h") => {
            println!(
                "gramly            run (menu bar app)\n\
                 gramly check      print suggestions for text on stdin\n\
                 gramly check --local   the on-device pass only, nothing sent\n\
                 gramly check --improve   the clearer wording the app proposes\n\
                 gramly doctor     what Gramly can see in the focused field (macOS)\n\
                 gramly lab        audit accepting fixes in real text fields, off screen (macOS)\n\
                 gramly preview [dir]   draw every card and the dictionary to PNG (macOS)\n\
                 gramly --version"
            );
            Ok(())
        }
        Some(other) => bail!("unknown command {other:?} (try `gramly help`)"),
    }
}

#[cfg(target_os = "macos")]
fn run() -> Result<()> {
    macos::run()
}

#[cfg(not(target_os = "macos"))]
fn run() -> Result<()> {
    bail!("the Linux surface is not built yet; `gramly check` works everywhere")
}

/// The model's fixes for `text`, asked a paragraph at a time as the app
/// asks: sent whole, Coda's text came back without its blank lines, the
/// lines no longer matched, and nothing was shown.
fn by_paragraph(
    text: &str,
    ask: impl Fn(&str) -> Result<Vec<gramly_core::Fix>>,
) -> Result<Vec<gramly_core::Fix>> {
    let mut fixes = Vec::new();
    for (offset, paragraph) in gramly_core::worker::asked(text) {
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
