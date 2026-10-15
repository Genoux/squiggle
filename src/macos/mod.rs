/// A line in `~/Library/Logs/gramly.log`, with the time: "it happened
/// again" can only be matched to a log line that says when.
macro_rules! note {
    ($($arg:tt)*) => {
        $crate::macos::note_line(&format!($($arg)*))
    };
}

mod app;
mod ax;
mod dictionary;
mod lab;
mod overlay;
mod paste;
mod preview;
mod theme;
mod write;

pub use app::run;
pub use lab::{host as lab_host, run as lab};
pub use preview::run as preview;

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
    if let Some(pid) = ax::frontmost_pid() {
        ax::wake(pid);
        // Electron builds its tree on the request; give it a moment.
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    if std::env::args().any(|a| a == "--raw") {
        println!("{}", ax::describe_focus());
        print!("{}", ax::probe_children());
        for (at, len) in [(0, 2), (4, 2)] {
            println!("raw bounds({at},{len}) {}", ax::probe_bounds(at, len));
        }
    }
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

pub(crate) fn note_line(message: &str) {
    use objc2_foundation::{NSDate, NSDateFormatter, NSString};
    let formatter = NSDateFormatter::new();
    formatter.setDateFormat(Some(&NSString::from_str("yyyy-MM-dd HH:mm:ss.SSS")));
    let now = formatter.stringFromDate(&NSDate::now());
    eprintln!("{now} gramly: {message}");
}
