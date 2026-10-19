mod app;
mod ax;
mod dictionary;
mod host;
mod lab;
mod overlay;
mod paste;
mod preview;
mod theme;

use crate::surface::{self, Field as _, Rect};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};

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
    print!("{}", surface::config_lines(&config));
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
                match field.bounds_utf16(0, first.max(1)) {
                    Some(rect) => format!("supported ({rect:?})"),
                    None => "NOT reported by this app - underlines cannot be placed".into(),
                }
            );
        }
    }
    Ok(())
}

impl From<CGRect> for Rect {
    fn from(rect: CGRect) -> Self {
        Rect {
            x: rect.origin.x,
            y: rect.origin.y,
            w: rect.size.width,
            h: rect.size.height,
        }
    }
}

impl From<Rect> for CGRect {
    fn from(rect: Rect) -> Self {
        CGRect::new(CGPoint::new(rect.x, rect.y), CGSize::new(rect.w, rect.h))
    }
}

/// The height of the primary display, which Accessibility measures y down
/// from and Cocoa measures y up from.
pub(crate) fn primary_height(mtm: objc2::MainThreadMarker) -> f64 {
    objc2_app_kit::NSScreen::screens(mtm)
        .firstObject()
        .map(|primary| primary.frame().size.height)
        .unwrap_or(0.0)
}
