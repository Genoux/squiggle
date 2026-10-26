use crate::surface::{Point, Rect};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_millis(150);

/// The front window, as Hyprland reports it: logical pixels, top-left origin.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub address: String,
    pub pid: u32,
    pub class: String,
    pub at: (f64, f64),
    pub size: (f64, f64),
}

/// A screen in the layout: its whole frame and the part panels and bars
/// leave free, both in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub frame: Rect,
    pub usable: Rect,
}

/// Matches the layers Gramly draws, by namespace, so Hyprland does not fade
/// them in and out: nothing animates.
const NO_ANIMATION: &str = r#"dispatch function() hl.layer_rule({ name = "gramly-still", match = { namespace = "^gramly-.*$" }, no_anim = true }) end"#;

/// The dictionary window's title, which its rule matches. A title and not a
/// class: GTK takes the class from the program for every plain window, so
/// the OpenRouter key window would share it.
pub const DICTIONARY_TITLE: &str = "Gramly dictionary";

/// A small utility window floats in the middle of the screen; tiled, it
/// would take half of it.
fn float_dictionary_command() -> String {
    format!(
        r#"dispatch function() hl.window_rule({{ name = "gramly-dictionary", match = {{ title = "^{DICTIONARY_TITLE}$" }}, float = true, center = true }}) end"#
    )
}

pub fn active_window() -> Option<Window> {
    parse(&request("j/activewindow")?)
}

pub fn monitors() -> Vec<Monitor> {
    request("j/monitors").map_or_else(Vec::new, |json| parse_monitors(&json))
}

pub fn pointer() -> Option<Point> {
    parse_pointer(&request("j/cursorpos")?)
}

fn send_rules() {
    [
        ("no-animation", NO_ANIMATION.to_string()),
        ("floating dictionary", float_dictionary_command()),
    ]
    .into_iter()
    .map(|(rule, command)| (rule, request(&command)))
    .filter(|(_, reply)| reply.as_deref().map(str::trim) != Some("ok"))
    .for_each(|(rule, reply)| note!("Hyprland did not take the {rule} rule: {reply:?}"));
}

/// Sends Gramly's rules now and again after every config reload, on a
/// thread of its own. A rule sent over the socket lasts only until the next
/// reload; after one, the user's `layersIn` slide moved every underline and
/// card into place.
pub fn keep_rules() {
    send_rules();
    let Some(events) = socket(".socket2.sock") else {
        return;
    };
    std::thread::spawn(move || {
        let Ok(stream) = UnixStream::connect(events) else {
            note!("not following Hyprland's reloads: no event socket");
            return;
        };
        BufReader::new(stream)
            .lines()
            .map_while(Result::ok)
            .filter(|event| event.starts_with("configreloaded>>"))
            .for_each(|_| send_rules());
    });
}

/// XKB keycodes, which is what Hyprland's `code:N` takes (evdev + 8: V is
/// evdev 47, Z is 44, Backspace 14). A keycode and not a keysym name, so the
/// shortcut lands on the same physical key whatever layout is active.
pub const KEY_V: u32 = 55;
pub const KEY_Z: u32 = 52;
pub const KEY_BACKSPACE: u32 = 22;

/// The dispatch that sends `mods`+`keycode` to the window at `address`.
/// Hyprland 0.56 takes its dispatchers as Lua.
fn shortcut_command(mods: &str, keycode: u32, address: &str) -> String {
    format!(
        r#"dispatch hl.dsp.send_shortcut({{ mods = "{mods}", key = "code:{keycode}", window = "address:{address}" }})"#
    )
}

/// Whether `front` is still the window the field was found in. The address
/// is what tells two windows of one app apart.
pub fn still_front(field: &Window, front: Option<&Window>) -> bool {
    front.is_some_and(|front| front.address == field.address && front.pid == field.pid)
}

/// Sends a shortcut to `window`, which is how a paste or an undo reaches an
/// app on Wayland, where no client may type into another.
pub fn send_shortcut(mods: &str, keycode: u32, window: &Window) -> bool {
    request(&shortcut_command(mods, keycode, &window.address))
        .is_some_and(|reply| reply.trim() == "ok")
}

fn socket(name: &str) -> Option<PathBuf> {
    Some(
        [
            std::env::var_os("XDG_RUNTIME_DIR")?,
            "hypr".into(),
            std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?,
            name.into(),
        ]
        .iter()
        .collect(),
    )
}

fn request(command: &str) -> Option<String> {
    let mut stream = UnixStream::connect(socket(".socket.sock")?).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    stream.write_all(command.as_bytes()).ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    Some(reply)
}

fn parse(json: &str) -> Option<Window> {
    let window: serde_json::Value = serde_json::from_str(json).ok()?;
    let pair = |key: &str| {
        let [x, y] = window[key].as_array()?.as_slice() else {
            return None;
        };
        Some((x.as_f64()?, y.as_f64()?))
    };
    Some(Window {
        address: window["address"].as_str()?.to_owned(),
        pid: u32::try_from(window["pid"].as_u64()?).ok()?,
        class: window["class"].as_str()?.to_owned(),
        at: pair("at")?,
        size: pair("size")?,
    })
}

fn parse_pointer(json: &str) -> Option<Point> {
    let pointer: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(Point {
        x: pointer["x"].as_f64()?,
        y: pointer["y"].as_f64()?,
    })
}

fn parse_monitors(json: &str) -> Vec<Monitor> {
    let Ok(serde_json::Value::Array(monitors)) = serde_json::from_str(json) else {
        return Vec::new();
    };
    monitors.iter().filter_map(monitor).collect()
}

fn monitor(entry: &serde_json::Value) -> Option<Monitor> {
    let number = |key: &str| entry[key].as_f64();
    let (w, h) = logical_size(
        (number("width")?, number("height")?),
        number("scale")?,
        entry["transform"].as_u64()?,
    );
    let frame = Rect {
        x: number("x")?,
        y: number("y")?,
        w,
        h,
    };
    let [left, top, right, bottom] = entry["reserved"].as_array()?.as_slice() else {
        return None;
    };
    let (left, top) = (left.as_f64()?, top.as_f64()?);
    Some(Monitor {
        name: entry["name"].as_str()?.to_owned(),
        frame,
        usable: Rect {
            x: frame.x + left,
            y: frame.y + top,
            w: frame.w - left - right.as_f64()?,
            h: frame.h - top - bottom.as_f64()?,
        },
    })
}

/// Hyprland reports a monitor's physical pixels; the layout is in logical
/// ones, and a quarter turn (transforms 1, 3, 5, 7) swaps the sides.
fn logical_size((width, height): (f64, f64), scale: f64, transform: u64) -> (f64, f64) {
    let (width, height) = (width / scale, height / scale);
    if transform % 2 == 1 {
        (height, width)
    } else {
        (width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `hyprctl activewindow -j` from Hyprland 0.56, with the title and
    /// address replaced.
    const SAMPLE: &str = r#"{
        "address": "0x1", "mapped": true, "hidden": false, "visible": true,
        "at": [963, 5], "size": [952, 1032],
        "workspace": {"id": 3, "name": "3"}, "floating": false, "monitor": 0,
        "class": "kitty", "title": "kitty", "initialClass": "kitty",
        "initialTitle": "kitty", "pid": 1758637, "xwayland": false,
        "pinned": false, "fullscreen": 0, "grouped": [], "tags": [],
        "focusHistoryID": 0, "stableId": "18000386"
    }"#;

    #[test]
    fn parses_a_real_reply() {
        assert_eq!(
            parse(SAMPLE),
            Some(Window {
                address: "0x1".into(),
                pid: 1758637,
                class: "kitty".into(),
                at: (963.0, 5.0),
                size: (952.0, 1032.0),
            })
        );
    }

    #[test]
    fn no_window_is_an_empty_object() {
        assert_eq!(parse("{}"), None);
    }

    #[test]
    fn garbage_is_none() {
        assert_eq!(parse("invalid"), None);
        assert_eq!(
            parse(r#"{"pid": 1, "class": "x", "at": [0], "size": [1, 2]}"#),
            None
        );
    }

    /// `hyprctl monitors -j` from Hyprland 0.56, trimmed to what is read.
    const MONITORS: &str = r#"[{
        "id": 0, "name": "DP-3", "width": 1920, "height": 1080,
        "x": 0, "y": 0, "reserved": [0, 0, 0, 38], "scale": 1,
        "transform": 0, "focused": true
    }]"#;

    #[test]
    fn parses_a_real_monitor_list() {
        assert_eq!(
            parse_monitors(MONITORS),
            [Monitor {
                name: "DP-3".into(),
                frame: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1920.0,
                    h: 1080.0
                },
                usable: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1920.0,
                    h: 1042.0
                },
            }]
        );
    }

    #[test]
    fn physical_size_becomes_logical() {
        let scaled = r#"[{"name": "eDP-1", "width": 2880, "height": 1800, "x": 10,
            "y": 20, "reserved": [0, 30, 0, 0], "scale": 2, "transform": 0}]"#;
        let [monitor] = parse_monitors(scaled).try_into().unwrap();
        assert_eq!(
            monitor.frame,
            Rect {
                x: 10.0,
                y: 20.0,
                w: 1440.0,
                h: 900.0
            }
        );
        assert_eq!(
            monitor.usable,
            Rect {
                x: 10.0,
                y: 50.0,
                w: 1440.0,
                h: 870.0
            }
        );
    }

    #[test]
    fn a_quarter_turn_swaps_width_and_height() {
        let turned = r#"[{"name": "DP-1", "width": 1080, "height": 1920, "x": 0,
            "y": 0, "reserved": [0, 0, 0, 0], "scale": 1, "transform": 1}]"#;
        let [monitor] = parse_monitors(turned).try_into().unwrap();
        assert_eq!((monitor.frame.w, monitor.frame.h), (1920.0, 1080.0));
        assert_eq!(logical_size((1080.0, 1920.0), 1.0, 2), (1080.0, 1920.0));
        assert_eq!(logical_size((1080.0, 1920.0), 1.0, 7), (1920.0, 1080.0));
    }

    #[test]
    fn a_broken_monitor_is_left_out() {
        assert_eq!(parse_monitors(r#"[{"name": "x"}]"#), []);
        assert_eq!(parse_monitors("invalid"), []);
    }

    #[test]
    fn the_dictionary_window_floats_centred_by_title() {
        assert_eq!(
            float_dictionary_command(),
            r#"dispatch function() hl.window_rule({ name = "gramly-dictionary", match = { title = "^Gramly dictionary$" }, float = true, center = true }) end"#
        );
    }

    #[test]
    fn a_shortcut_names_the_window_by_address_and_the_key_by_keycode() {
        assert_eq!(
            shortcut_command("CTRL", KEY_V, "0x55d0c8a1b2c0"),
            r#"dispatch hl.dsp.send_shortcut({ mods = "CTRL", key = "code:55", window = "address:0x55d0c8a1b2c0" })"#
        );
        assert_eq!(
            shortcut_command("CTRL", KEY_Z, "0x1"),
            r#"dispatch hl.dsp.send_shortcut({ mods = "CTRL", key = "code:52", window = "address:0x1" })"#
        );
        assert_eq!(
            shortcut_command("", KEY_BACKSPACE, "0x1"),
            r#"dispatch hl.dsp.send_shortcut({ mods = "", key = "code:22", window = "address:0x1" })"#
        );
    }

    fn window(address: &str, pid: u32) -> Window {
        Window {
            address: address.into(),
            pid,
            class: "zen".into(),
            at: (0.0, 0.0),
            size: (1.0, 1.0),
        }
    }

    #[test]
    fn only_the_same_window_is_still_in_front() {
        let field = window("0x1", 7);
        assert!(still_front(&field, Some(&window("0x1", 7))));
        assert!(!still_front(&field, Some(&window("0x2", 7))));
        assert!(!still_front(&field, Some(&window("0x1", 8))));
        assert!(!still_front(&field, None));
    }

    #[test]
    fn parses_the_pointer() {
        assert_eq!(
            parse_pointer(r#"{"x": 452, "y": 884}"#),
            Some(Point { x: 452.0, y: 884.0 })
        );
        assert_eq!(parse_pointer("{}"), None);
    }
}
