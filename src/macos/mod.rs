mod app;
mod ax;
mod overlay;
mod paste;
mod theme;

pub use app::run;

/// What Gramly can see right now, for TROUBLESHOOTING.md. Waits a moment so
/// there is time to click into the field in question.
pub fn doctor() -> anyhow::Result<()> {
    let config = gramly_core::Config::load()?;
    println!("gramly {}", env!("CARGO_PKG_VERSION"));
    println!(
        "accessibility    {}",
        if ax::trusted(false) {
            "allowed"
        } else {
            "NOT allowed (System Settings > Privacy & Security > Accessibility)"
        }
    );
    println!(
        "checking         {}",
        if config.enabled { "on" } else { "paused" }
    );
    println!(
        "ai suggestions   {}",
        match (config.ai, config.openrouter_key.is_some()) {
            (true, true) => "on (key set)",
            (true, false) => "off - no OpenRouter key",
            (false, _) => "off",
        }
    );
    println!("\nclick into a text field; looking in 3 seconds...");
    std::thread::sleep(std::time::Duration::from_secs(3));
    match ax::explain_focus() {
        Err(reason) => println!("focused field    none: {reason}"),
        Ok(field) => {
            let text = field.value().unwrap_or_default();
            println!(
                "focused field    found, {} characters",
                text.chars().count()
            );
            println!("field frame      {:?}", field.frame());
            println!("visible part     {:?}", field.visible_frame());
            let first = text.encode_utf16().take(5).count();
            println!(
                "word positions   {}",
                match field.bounds(0, first.max(1)) {
                    Some(rect) => format!("supported ({rect:?})"),
                    None => "NOT reported by this app - underlines cannot be placed".into(),
                }
            );
        }
    }
    Ok(())
}
