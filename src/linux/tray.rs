//! The tray icon and its menu, over StatusNotifierItem. ksni serves D-Bus on
//! its own thread, so a click only sends a [`Command`]; the GTK main loop
//! applies it, and the engine is never touched from here.

use crate::surface::{DIALECTS, Status};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{MenuItem, Tray};
use std::sync::mpsc::Sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ToggleEnabled,
    Pause,
    ToggleApp,
    ToggleAi,
    SetKey,
    Spelling(usize),
    Dictionary,
    CopyLog,
    Quit,
}

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

fn plain(label: impl Into<String>, enabled: bool, command: Command) -> MenuItem<GramlyTray> {
    StandardItem {
        label: label.into(),
        enabled,
        activate: send(command),
        ..Default::default()
    }
    .into()
}

fn note(label: impl Into<String>) -> MenuItem<GramlyTray> {
    StandardItem {
        label: label.into(),
        enabled: false,
        ..Default::default()
    }
    .into()
}

fn checkmark(label: &str, checked: bool, command: Command) -> MenuItem<GramlyTray> {
    CheckmarkItem {
        label: label.into(),
        checked,
        activate: send(command),
        ..Default::default()
    }
    .into()
}

impl GramlyTray {
    fn new(status: Status, commands: Sender<Command>) -> Self {
        Self { status, commands }
    }

    fn spelling(&self) -> MenuItem<Self> {
        let commands = self.commands.clone();
        let current = DIALECTS
            .iter()
            .position(|(dialect, _)| *dialect == self.status.dialect)
            .unwrap_or(0);
        SubMenu {
            label: "Spelling".into(),
            submenu: vec![
                RadioGroup {
                    selected: current,
                    select: Box::new(move |_, index| {
                        let _ = commands.send(Command::Spelling(index));
                    }),
                    options: DIALECTS
                        .iter()
                        .map(|(_, name)| RadioItem {
                            label: (*name).into(),
                            ..Default::default()
                        })
                        .collect(),
                }
                .into(),
            ],
            ..Default::default()
        }
        .into()
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
        let status = &self.status;
        let app = status.app_item();
        vec![
            note(&status.line),
            MenuItem::Separator,
            checkmark("Check as I type", status.enabled, Command::ToggleEnabled),
            plain(status.pause_title(), status.enabled, Command::Pause),
            StandardItem {
                label: app
                    .as_ref()
                    .map_or_else(String::new, |app| app.title.clone()),
                enabled: app.as_ref().is_some_and(|app| app.enabled),
                visible: app.is_some(),
                activate: send(Command::ToggleApp),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            checkmark("AI suggestions", status.ai, Command::ToggleAi),
            plain("OpenRouter key…", true, Command::SetKey),
            self.spelling(),
            plain("Personal dictionary…", true, Command::Dictionary),
            MenuItem::Separator,
            plain("Copy recent log", true, Command::CopyLog),
            note(concat!("Gramly ", env!("CARGO_PKG_VERSION"))),
            plain("Quit Gramly", true, Command::Quit),
        ]
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
    use crate::surface::Status;
    use ksni::{Tray, menu::MenuItem};
    use std::sync::mpsc;

    fn status() -> Status {
        Status {
            line: "Checking on this computer only".into(),
            trusted: true,
            enabled: true,
            ai: false,
            paused: false,
            front: Some(("firefox".into(), "firefox".into())),
            off_here: false,
            terminal: false,
            dialect: "british".into(),
        }
    }

    fn tray(status: Status) -> (GramlyTray, mpsc::Receiver<Command>) {
        let (sender, commands) = mpsc::channel();
        (GramlyTray::new(status, sender), commands)
    }

    fn labels(tray: &GramlyTray) -> Vec<String> {
        tray.menu()
            .iter()
            .filter_map(|item| match item {
                MenuItem::Standard(item) if item.visible => Some(item.label.clone()),
                MenuItem::Checkmark(item) => Some(item.label.clone()),
                MenuItem::SubMenu(item) => Some(item.label.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_menu_has_the_macos_items_in_order_and_no_access_item() {
        let (tray, _) = tray(status());
        assert_eq!(
            labels(&tray),
            [
                "Checking on this computer only",
                "Check as I type",
                "Pause for an hour",
                "Turn off in firefox",
                "AI suggestions",
                "OpenRouter key…",
                "Spelling",
                "Personal dictionary…",
                "Copy recent log",
                concat!("Gramly ", env!("CARGO_PKG_VERSION")),
                "Quit Gramly",
            ]
        );
    }

    #[test]
    fn the_app_item_is_hidden_with_no_app_in_front() {
        let (tray, _) = tray(Status {
            front: None,
            ..status()
        });
        assert!(!labels(&tray).iter().any(|label| label.contains("Turn")));
    }

    #[test]
    fn pause_follows_the_status() {
        let (tray, _) = tray(Status {
            paused: true,
            enabled: false,
            ..status()
        });
        let menu = tray.menu();
        let MenuItem::Standard(pause) = menu
            .iter()
            .find(
                |item| matches!(item, MenuItem::Standard(item) if item.label == "Resume checking"),
            )
            .expect("pause says resume")
        else {
            unreachable!();
        };
        assert!(!pause.enabled);
    }

    #[test]
    fn spelling_selects_the_current_dialect() {
        let (tray, _) = tray(status());
        let menu = tray.menu();
        let Some(MenuItem::SubMenu(spelling)) = menu
            .iter()
            .find(|item| matches!(item, MenuItem::SubMenu(_)))
        else {
            panic!("no spelling submenu");
        };
        let MenuItem::RadioGroup(group) = &spelling.submenu[0] else {
            panic!("spelling is a radio group");
        };
        assert_eq!(group.selected, 1);
        assert_eq!(group.options.len(), crate::surface::DIALECTS.len());
    }

    #[test]
    fn clicking_an_item_sends_its_command() {
        let (mut tray, commands) = tray(status());
        let menu = tray.menu();
        let Some(MenuItem::Checkmark(enabled)) = menu.iter().find(
            |item| matches!(item, MenuItem::Checkmark(item) if item.label == "Check as I type"),
        ) else {
            panic!("check as I type is a checkmark");
        };
        (enabled.activate)(&mut tray);
        assert_eq!(commands.try_recv(), Ok(Command::ToggleEnabled));
    }
}
