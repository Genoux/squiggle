//! The status menu, decided once. Each platform draws these items and sends
//! the [`Command`] of whichever is clicked back to the run loop; neither
//! decides what is listed, in what order, or when it is enabled.

use super::{DIALECTS, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Pause,
    ToggleApp,
    ToggleAi,
    SetKey,
    Spelling(usize),
    Dictionary,
    CopyLog,
    /// Only offered while untrusted, which only macOS can be.
    GrantAccess,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Note(String),
    Separator,
    Action {
        label: String,
        enabled: bool,
        command: Command,
    },
    Check {
        label: String,
        on: bool,
        command: Command,
    },
    Submenu {
        label: String,
        items: Vec<Item>,
    },
}

fn action(label: impl Into<String>, command: Command) -> Item {
    Item::Action {
        label: label.into(),
        enabled: true,
        command,
    }
}

pub fn menu(status: &Status) -> Vec<Item> {
    let grant =
        (!status.trusted).then(|| action("Grant Accessibility access…", Command::GrantAccess));
    let app = status.app_item().map(|app| Item::Action {
        label: app.title,
        enabled: app.enabled,
        command: Command::ToggleApp,
    });
    let spelling = DIALECTS
        .iter()
        .enumerate()
        .map(|(index, (dialect, name))| Item::Check {
            label: (*name).into(),
            on: status.dialect == *dialect,
            command: Command::Spelling(index),
        })
        .collect();
    [
        Some(Item::Note(status.line.clone())),
        grant,
        Some(Item::Separator),
        Some(action(status.pause_title(), Command::Pause)),
        app,
        Some(Item::Separator),
        Some(Item::Check {
            label: "AI suggestions".into(),
            on: status.ai,
            command: Command::ToggleAi,
        }),
        Some(action("OpenRouter key…", Command::SetKey)),
        Some(Item::Submenu {
            label: "Spelling".into(),
            items: spelling,
        }),
        Some(action("Personal dictionary…", Command::Dictionary)),
        Some(Item::Separator),
        Some(action("Copy recent log", Command::CopyLog)),
        Some(action(
            concat!("Quit Gramly ", env!("CARGO_PKG_VERSION")),
            Command::Quit,
        )),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status {
            line: "Gramly is on".into(),
            trusted: true,
            ai: true,
            paused: false,
            front: Some(("com.slack.Slack".into(), "Slack".into())),
            off_here: false,
            terminal: false,
            dialect: "british".into(),
        }
    }

    fn labels(items: &[Item]) -> Vec<&str> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Note(label)
                | Item::Action { label, .. }
                | Item::Check { label, .. }
                | Item::Submenu { label, .. } => Some(label.as_str()),
                Item::Separator => None,
            })
            .collect()
    }

    fn with(change: impl FnOnce(&mut Status)) -> Vec<Item> {
        let mut status = status();
        change(&mut status);
        menu(&status)
    }

    const QUIT: &str = concat!("Quit Gramly ", env!("CARGO_PKG_VERSION"));

    #[test]
    fn the_menu_is_in_order() {
        let items = menu(&status());
        assert_eq!(
            labels(&items),
            [
                "Gramly is on",
                "Pause for an hour",
                "Turn off in Slack",
                "AI suggestions",
                "OpenRouter key…",
                "Spelling",
                "Personal dictionary…",
                "Copy recent log",
                QUIT,
            ]
        );
        let separators: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| **item == Item::Separator)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(separators, [1, 4, 9]);
    }

    #[test]
    fn spelling_checks_the_current_dialect() {
        let items = menu(&status());
        let Some(Item::Submenu { items, .. }) = items
            .iter()
            .find(|item| matches!(item, Item::Submenu { .. }))
        else {
            panic!("no spelling submenu");
        };
        let on: Vec<Command> = items
            .iter()
            .filter_map(|item| match item {
                Item::Check {
                    on: true, command, ..
                } => Some(*command),
                _ => None,
            })
            .collect();
        assert_eq!(on, [Command::Spelling(1)]);
        assert_eq!(items.len(), DIALECTS.len());
    }

    #[test]
    fn paused_offers_resume() {
        let items = with(|s| s.paused = true);
        assert_eq!(labels(&items)[1], "Resume");
    }

    #[test]
    fn off_here_offers_turning_on() {
        let items = with(|s| s.off_here = true);
        assert_eq!(labels(&items)[2], "Turn on in Slack");
    }

    #[test]
    fn a_terminal_is_disabled_with_its_reason() {
        let items = with(|s| s.terminal = true);
        assert!(items.contains(&Item::Action {
            label: "Off in Slack: terminals are skipped".into(),
            enabled: false,
            command: Command::ToggleApp,
        }));
    }

    #[test]
    fn no_app_in_front_drops_the_app_item() {
        let items = with(|s| s.front = None);
        assert!(!labels(&items).iter().any(|label| label.contains("Slack")));
        assert_eq!(labels(&items).len(), 8);
    }

    #[test]
    fn ai_off_unchecks_the_toggle() {
        let items = with(|s| s.ai = false);
        assert!(items.contains(&Item::Check {
            label: "AI suggestions".into(),
            on: false,
            command: Command::ToggleAi,
        }));
    }

    #[test]
    fn the_status_line_is_shown_as_given() {
        let items = with(|s| s.line = "AI unavailable, checking on this computer".into());
        assert_eq!(
            items[0],
            Item::Note("AI unavailable, checking on this computer".into())
        );
    }

    #[test]
    fn untrusted_offers_access_after_the_status() {
        let items = with(|s| s.trusted = false);
        assert_eq!(labels(&items)[1], "Grant Accessibility access…");
    }
}
