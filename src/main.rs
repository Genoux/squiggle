//! Gramly: grammar suggestions in whatever text field has focus.
//!
//! `gramly` runs the app. `gramly check` reads text on stdin and prints what
//! would be underlined - the same engine, without a screen, for debugging a
//! suggestion and for scripts.

#[cfg(target_os = "macos")]
mod macos;

use anyhow::{Result, bail};
use gramly_core::{Config, Local, ai, config, merge};
use std::io::Read;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("run") => run(),
        Some("check") => check(args.iter().any(|a| a == "--local")),
        #[cfg(target_os = "macos")]
        Some("doctor") => macos::doctor(),
        Some("--version" | "-V") => {
            println!("gramly {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("help" | "--help" | "-h") => {
            println!(
                "gramly            run (menu bar app)\n\
                 gramly check      print suggestions for text on stdin\n\
                 gramly check --local   the on-device pass only, nothing sent\n\
                 gramly doctor     what Gramly can see in the focused field (macOS)\n\
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

fn check(local_only: bool) -> Result<()> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let config = Config::load()?;
    let mut local = Local::new(config.harper_dialect());
    local.set_known_words(config::dictionary());
    let found = local.check(&text);
    let model = match (&config.openrouter_key, config.ai && !local_only) {
        (Some(key), true) => ai::check(key, &text)?,
        _ => Vec::new(),
    };
    for fix in merge(found, model) {
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
