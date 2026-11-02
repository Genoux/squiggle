//! The tray icon, over StatusNotifierItem, drawing `surface::menu`. ksni
//! serves D-Bus on its own thread, so a click only sends a [`Command`]; the
//! GTK main loop applies it, and the engine is never touched from here.

use crate::surface::Status;
use crate::surface::menu::{Command, Item, menu};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{MenuItem, Tray};
use std::sync::mpsc::Sender;

pub struct GramlyTray {
    status: Status,
    commands: Sender<Command>,
}

type Action = Box<dyn Fn(&mut GramlyTray) + Send>;

fn send(command: Command) -> Action {
    Box::new(move |tray| {
        // The receiver is gone only while the app is quitting.
        let _ = tray.commands.send(command);
    })
}

fn render(item: Item) -> MenuItem<GramlyTray> {
    match item {
        Item::Note(label) => StandardItem {
            label,
            enabled: false,
            ..Default::default()
        }
        .into(),
        Item::Separator => MenuItem::Separator,
        Item::Action {
            label,
            enabled,
            command,
        } => StandardItem {
            label,
            enabled,
            activate: send(command),
            ..Default::default()
        }
        .into(),
        Item::Check { label, on, command } => CheckmarkItem {
            label,
            checked: on,
            activate: send(command),
            ..Default::default()
        }
        .into(),
        Item::Submenu { label, items } => SubMenu {
            label,
            submenu: items.into_iter().map(render).collect(),
            ..Default::default()
        }
        .into(),
    }
}

impl GramlyTray {
    fn new(status: Status, commands: Sender<Command>) -> Self {
        Self { status, commands }
    }
}

impl Tray for GramlyTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "gramly".into()
    }

    fn title(&self) -> String {
        "Gramly".into()
    }

    fn icon_name(&self) -> String {
        "tools-check-spelling".into()
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        menu(&self.status).into_iter().map(render).collect()
    }
}

pub struct TrayHandle(Handle<GramlyTray>);

impl TrayHandle {
    pub fn show(&self, status: Status) {
        self.0.update(|tray| tray.status = status);
    }
}

/// Spawned even when no tray host is up yet: at login Gramly can start
/// before the shell, and the icon should appear when the host does rather
/// than never.
pub fn spawn(status: Status, commands: Sender<Command>) -> anyhow::Result<TrayHandle> {
    let handle = GramlyTray::new(status, commands)
        .assume_sni_available(true)
        .spawn()?;
    Ok(TrayHandle(handle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn status() -> Status {
        Status {
            trusted: true,
            ai: false,
            paused: None,
            blocked: None,
            off: Vec::new(),
            dialect: "british".into(),
        }
    }

    fn tray() -> (GramlyTray, mpsc::Receiver<Command>) {
        let (sender, commands) = mpsc::channel();
        (GramlyTray::new(status(), sender), commands)
    }

    fn label(item: &MenuItem<GramlyTray>) -> Option<&str> {
        match item {
            MenuItem::Standard(item) => Some(&item.label),
            MenuItem::Checkmark(item) => Some(&item.label),
            MenuItem::SubMenu(item) => Some(&item.label),
            _ => None,
        }
    }

    #[test]
    fn the_tray_draws_the_shared_menu_in_order() {
        let (tray, _) = tray();
        let drawn = tray.menu();
        let shared = menu(&status());
        assert_eq!(drawn.len(), shared.len());
        let drawn_separators: Vec<bool> = drawn
            .iter()
            .map(|item| matches!(item, MenuItem::Separator))
            .collect();
        let shared_separators: Vec<bool> =
            shared.iter().map(|item| *item == Item::Separator).collect();
        assert_eq!(drawn_separators, shared_separators);
        let names: Vec<&str> = drawn.iter().filter_map(label).collect();
        assert_eq!(names[0], concat!("Gramly ", env!("CARGO_PKG_VERSION")));
        assert_eq!(names[1], "Pause");
        assert_eq!(names.last(), Some(&"Quit Gramly"));
    }

    #[test]
    fn the_note_is_disabled() {
        let (tray, _) = tray();
        let menu = tray.menu();
        let MenuItem::Standard(note) = &menu[0] else {
            panic!("the name is a standard item");
        };
        assert!(!note.enabled);
    }

    #[test]
    fn clicking_an_item_sends_its_command() {
        let (mut tray, commands) = tray();
        let menu = tray.menu();
        let Some(MenuItem::Checkmark(ai)) = menu
            .iter()
            .find(|item| label(item) == Some("AI suggestions"))
        else {
            panic!("AI suggestions is a checkmark");
        };
        (ai.activate)(&mut tray);
        assert_eq!(commands.try_recv(), Ok(Command::ToggleAi));
    }

    #[test]
    fn the_spelling_submenu_is_rendered_with_its_choices() {
        let (tray, _) = tray();
        let menu = tray.menu();
        let Some(MenuItem::SubMenu(spelling)) =
            menu.iter().find(|item| label(item) == Some("Spelling"))
        else {
            panic!("no spelling submenu");
        };
        assert_eq!(spelling.submenu.len(), crate::surface::DIALECTS.len());
    }
}
