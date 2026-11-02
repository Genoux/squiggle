mod actions;
mod appearance;
mod atspi;
mod card;
mod clipboard;
mod dictionary;
mod edit;
mod font;
mod host;
mod hypr;
mod key;
mod overlay;
mod tray;

use crate::surface::menu::Command;
use crate::surface::{self, Engine, Field as _, Status, TICK};
use gtk::glib;
use gtk4 as gtk;
use host::LinuxHost;
use overlay::LayerOverlay;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::mpsc;
use std::time::Duration;

type LinuxEngine = Engine<LinuxHost, LayerOverlay>;
type SharedEngine = Rc<RefCell<Option<LinuxEngine>>>;

pub fn run() -> anyhow::Result<()> {
    font::register();
    gtk::init()?;
    anyhow::ensure!(
        gtk4_layer_shell::is_supported(),
        "this compositor does not offer wlr-layer-shell; Gramly draws on it"
    );
    if !atspi::enable()? {
        note!("accessibility turned on now; apps started before this may need a restart");
    }
    let conn = atspi::connect()?;
    let focus = atspi::listen(&conn)?;
    hypr::keep_rules();

    let engine: SharedEngine = Rc::new(RefCell::new(None));
    let clicked = Rc::downgrade(&engine);
    let overlay = LayerOverlay::new(Rc::new(move |action, index| {
        // A click that arrives while the engine is mid-tick is dropped, as
        // on macOS.
        if let Some(engine) = clicked.upgrade()
            && let Ok(mut engine) = engine.try_borrow_mut()
            && let Some(engine) = engine.as_mut()
        {
            engine.act(action, index);
        }
    }));
    *engine.borrow_mut() = Some(Engine::new(LinuxHost::new(conn, focus), overlay));

    let (sender, commands) = mpsc::channel();
    let initial = engine.borrow().as_ref().map(Engine::status);
    let tray = initial.and_then(|status| {
        tray::spawn(status, sender)
            .inspect_err(|why| note!("no tray icon: {why:#}"))
            .ok()
    });

    let main_loop = glib::MainLoop::new(None, false);
    let quit = main_loop.clone();
    let shared = Rc::downgrade(&engine);
    let mut drawn: Option<Status> = None;
    glib::timeout_add_local(TICK, move || {
        if let Ok(mut engine) = engine.try_borrow_mut()
            && let Some(engine) = engine.as_mut()
        {
            engine.tick();
            commands
                .try_iter()
                .for_each(|command| apply(command, engine, &shared, &quit));
            let status = engine.status();
            if let Some(tray) = &tray
                && drawn.as_ref() != Some(&status)
            {
                tray.show(status.clone());
                drawn = Some(status);
            }
        }
        glib::ControlFlow::Continue
    });
    note!("{} ready", env!("CARGO_PKG_VERSION"));
    main_loop.run();
    Ok(())
}

fn apply(
    command: Command,
    engine: &mut LinuxEngine,
    shared: &Weak<RefCell<Option<LinuxEngine>>>,
    main_loop: &glib::MainLoop,
) {
    match command {
        Command::PauseFor(index) => engine.pause_for(index),
        Command::Resume => engine.resume(),
        Command::TurnOn(index) => engine.turn_on(index),
        Command::TurnAllOn => engine.turn_all_on(),
        Command::AddCredits => {
            if let Err(err) = gtk::gio::AppInfo::launch_default_for_uri(
                surface::ADD_CREDITS,
                None::<&gtk::gio::AppLaunchContext>,
            ) {
                note!("opening {}: {err}", surface::ADD_CREDITS);
            }
        }
        Command::ToggleAi => engine.toggle_ai(),
        Command::SetKey => {
            let shared = shared.clone();
            key::ask(move || {
                with_engine(&shared, |engine| {
                    engine.clear_ai_error();
                    engine.reset();
                });
            });
        }
        Command::Spelling(index) => engine.set_dialect(index),
        Command::Dictionary => {
            let shared = shared.clone();
            dictionary::show(move || with_engine(&shared, LinuxEngine::recheck));
        }
        Command::CopyLog => actions::copy_recent_log(),
        Command::GrantAccess => {}
        Command::Quit => main_loop.quit(),
    }
}

/// Runs `change` on the engine from a window's callback, which can arrive
/// while the engine is mid-tick; that one is dropped, as a click on a card is.
fn with_engine(shared: &Weak<RefCell<Option<LinuxEngine>>>, change: impl FnOnce(&mut LinuxEngine)) {
    if let Some(engine) = shared.upgrade()
        && let Ok(mut engine) = engine.try_borrow_mut()
        && let Some(engine) = engine.as_mut()
    {
        change(engine);
    }
}

/// What Gramly can see right now, for TROUBLESHOOTING.md. Waits a moment so
/// there is time to click into the field in question.
pub fn doctor() -> anyhow::Result<()> {
    let config = gramly_core::Config::load()?;
    println!("gramly {}", env!("CARGO_PKG_VERSION"));
    let was_on = atspi::enable()?;
    println!(
        "accessibility    {}",
        if was_on {
            "on"
        } else {
            "turned on now - apps started before this may need a restart"
        }
    );
    print!("{}", surface::config_lines(&config));
    let conn = atspi::connect()?;
    let focus = atspi::listen(&conn)?;
    println!("\nclick into a text field; looking in 3 seconds...");
    std::thread::sleep(Duration::from_secs(3));
    let Some(window) = hypr::active_window() else {
        println!(
            "front window     none (is Hyprland running, with HYPRLAND_INSTANCE_SIGNATURE set?)"
        );
        return Ok(());
    };
    println!(
        "front window     {} (pid {}) at {:?} size {:?}",
        window.class, window.pid, window.at, window.size
    );
    match atspi::explain_focus(&conn, &focus, &window) {
        Err(reason) => println!("focused field    none: {reason}"),
        Ok((field, via)) => {
            let text = field.value().unwrap_or_default();
            println!(
                "focused field    found, {} characters, by {}",
                text.chars().count(),
                via.name()
            );
            println!("field frame      {:?}", field.visible_frame());
            let first = words(&text).first().copied().unwrap_or((0, 1));
            println!(
                "word positions   {}",
                match field.bounds(first.0, first.1) {
                    Some(rect) => format!("supported ({rect:?})"),
                    None => "NOT reported by this app - underlines cannot be placed".into(),
                }
            );
            println!(
                "fixes written    {}",
                if field.is_web() {
                    "pasted over a confirmed selection (web field)"
                } else {
                    "directly, then read back"
                }
            );
            if std::env::args().any(|a| a == "--raw") {
                println!("text             {text:?}");
                for (start, len) in words(&text) {
                    println!("raw bounds({start},{len}) {:?}", field.bounds(start, len));
                }
            }
        }
    }
    Ok(())
}

/// Where each whitespace-separated word starts and how long it is, in
/// characters.
fn words(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    chars
        .chunk_by(|a, b| a.is_whitespace() == b.is_whitespace())
        .map(|run| {
            let at = start;
            start += run.len();
            (at, run)
        })
        .filter(|(_, run)| !run[0].is_whitespace())
        .map(|(at, run)| (at, run.len()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_count_characters_not_bytes() {
        assert_eq!(words("Hello 😀 wrold"), [(0, 5), (6, 1), (8, 5)]);
    }

    #[test]
    fn words_skip_runs_of_whitespace() {
        assert_eq!(words("  a\n\nbc "), [(2, 1), (5, 2)]);
        assert_eq!(words(""), []);
    }
}
