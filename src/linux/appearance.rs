//! Light or dark, as the desktop says. The freedesktop portal's
//! `color-scheme` is the one answer every desktop gives and the one GTK
//! itself follows from 4.20; `gtk-application-prefer-dark-theme` only
//! reflects a setting an app made for itself and says nothing about the
//! system. It is followed on a thread of its own, so a slow portal never
//! stalls the main loop, and read by the card when it is built, as macOS
//! reads its appearance, and by the tray for its icon's colours.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// 1 is dark; 0 is no preference and 2 is light. Older portals wrapped the
/// value in a second variant.
fn is_dark(value: &Value) -> bool {
    match value {
        Value::U32(scheme) => *scheme == 1,
        Value::Value(inner) => is_dark(inner),
        _ => false,
    }
}

fn follow(dark: &AtomicBool) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let portal = Proxy::new(&connection, DESTINATION, PATH, INTERFACE)?;
    let signals = portal.receive_signal("SettingChanged")?;
    // `ReadOne` is Settings v2; a v1 portal answers only the deprecated
    // `Read`, and still sends `SettingChanged`.
    let current: OwnedValue = portal
        .call("ReadOne", &(NAMESPACE, KEY))
        .or_else(|_| portal.call("Read", &(NAMESPACE, KEY)))?;
    dark.store(is_dark(&current), Ordering::Relaxed);
    for signal in signals {
        let Ok((namespace, key, value)) =
            signal.body().deserialize::<(String, String, OwnedValue)>()
        else {
            continue;
        };
        if namespace == NAMESPACE && key == KEY {
            dark.store(is_dark(&value), Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Whether the desktop is dark now, kept up to date. Light until the portal
/// answers, and for good when there is none.
pub fn watch() -> Arc<AtomicBool> {
    let dark = Arc::new(AtomicBool::new(false));
    let shared = Arc::clone(&dark);
    std::thread::spawn(move || {
        if let Err(why) = follow(&shared) {
            note!("not following the desktop's light or dark: {why:#}");
        }
    });
    dark
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_means_dark() {
        assert!(is_dark(&Value::U32(1)));
        assert!(!is_dark(&Value::U32(0)));
        assert!(!is_dark(&Value::U32(2)));
    }

    #[test]
    fn a_wrapped_value_is_read_through() {
        assert!(is_dark(&Value::Value(Box::new(Value::U32(1)))));
    }
}
