//! The status menu, decided once. Each platform draws these items and sends
//! the [`Command`] of whichever is clicked back to the run loop; neither
//! decides what is listed, in what order, or when it is enabled.
//!
//! No status line and no on/off switch: running is on, quitting is off, and
//! why a model call failed is in the log. Ordered as most menu bar apps are:
//! the name, what Squiggle is doing, settings, then the log and Quit.

use super::{DIALECTS, PAUSES, Status};
use squiggle_core::router::Blocked;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// One of [`PAUSES`], by its place in the list.
    PauseFor(usize),
    Resume,
    /// Opens OpenRouter's credits page.
    AddCredits,
    /// A field turned off from a card, back on, by its place in
    /// `Status::off`.
    TurnOn(usize),
    TurnAllOn,
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
    let mut items = vec![
        Item::Note(concat!("Squiggle ", env!("CARGO_PKG_VERSION")).into()),
        Item::Separator,
    ];
    if !status.trusted {
        items.push(action("Grant Accessibility access…", Command::GrantAccess));
    }
    // The one status shown: the writer has to act, and the log was the only
    // place that said so.
    match status.blocked {
        Some(Blocked::Credits) => items.push(action(
            "AI suggestions stopped: add OpenRouter credits…",
            Command::AddCredits,
        )),
        Some(Blocked::Key) => items.push(action(
            "AI suggestions stopped: check your OpenRouter key…",
            Command::SetKey,
        )),
        None => {}
    }
    match &status.paused {
        Some(line) => {
            items.push(Item::Note(line.clone()));
            items.push(action("Resume", Command::Resume));
        }
        None => items.push(Item::Submenu {
            label: "Pause".into(),
            items: PAUSES
                .iter()
                .enumerate()
                .map(|(index, (_, name))| action(*name, Command::PauseFor(index)))
                .collect(),
        }),
    }
    if !status.off.is_empty() {
        let mut list = vec![Item::Note("Click one to suggest there again".into())];
        list.extend(
            status
                .off
                .iter()
                .enumerate()
                .map(|(index, name)| action(name.as_str(), Command::TurnOn(index))),
        );
        list.push(Item::Separator);
        list.push(action("Turn all back on", Command::TurnAllOn));
        items.push(Item::Submenu {
            label: match status.off.len() {
                1 => "Off in 1 field".into(),
                count => format!("Off in {count} fields"),
            },
            items: list,
        });
    }
    let spelling = DIALECTS
        .iter()
        .enumerate()
        .map(|(index, (dialect, name))| Item::Check {
            label: (*name).into(),
            on: status.dialect == *dialect,
            command: Command::Spelling(index),
        })
        .collect();
    items.extend([
        Item::Separator,
        Item::Check {
            label: "AI suggestions".into(),
            on: status.ai,
            command: Command::ToggleAi,
        },
        Item::Submenu {
            label: "Spelling".into(),
            items: spelling,
        },
        action("Personal dictionary…", Command::Dictionary),
        action("OpenRouter key…", Command::SetKey),
        Item::Separator,
        action("Copy recent log", Command::CopyLog),
        action("Quit Squiggle", Command::Quit),
    ]);
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status {
            trusted: true,
            ai: true,
            paused: None,
            blocked: None,
            off: Vec::new(),
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

    fn submenu<'a>(items: &'a [Item], wanted: &str) -> &'a [Item] {
        items
            .iter()
            .find_map(|item| match item {
                Item::Submenu { label, items } if label.starts_with(wanted) => {
                    Some(items.as_slice())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {wanted} submenu"))
    }

    const NAME: &str = concat!("Squiggle ", env!("CARGO_PKG_VERSION"));

    #[test]
    fn the_menu_is_in_order() {
        let items = menu(&status());
        assert_eq!(
            labels(&items),
            [
                NAME,
                "Pause",
                "AI suggestions",
                "Spelling",
                "Personal dictionary…",
                "OpenRouter key…",
                "Copy recent log",
                "Quit Squiggle",
            ]
        );
        let separators: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| **item == Item::Separator)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(separators, [1, 3, 8]);
    }

    #[test]
    fn pause_offers_each_length() {
        let items = menu(&status());
        let lengths = submenu(&items, "Pause");
        assert_eq!(labels(lengths).len(), PAUSES.len());
        assert_eq!(
            lengths.last(),
            Some(&action(
                "Until tomorrow",
                Command::PauseFor(PAUSES.len() - 1)
            ))
        );
    }

    #[test]
    fn paused_says_until_when_and_offers_resume() {
        let items = with(|s| s.paused = Some("Paused until 15:40".into()));
        assert_eq!(labels(&items)[1..3], ["Paused until 15:40", "Resume"]);
        assert!(!labels(&items).contains(&"Pause"));
    }

    #[test]
    fn spelling_checks_the_current_dialect() {
        let items = menu(&status());
        let on: Vec<Command> = submenu(&items, "Spelling")
            .iter()
            .filter_map(|item| match item {
                Item::Check {
                    on: true, command, ..
                } => Some(*command),
                _ => None,
            })
            .collect();
        assert_eq!(on, [Command::Spelling(1)]);
    }

    #[test]
    fn out_of_credits_leads_to_credits_and_a_bad_key_to_the_key() {
        let items = with(|s| s.blocked = Some(Blocked::Credits));
        assert_eq!(
            items[2],
            action(
                "AI suggestions stopped: add OpenRouter credits…",
                Command::AddCredits
            )
        );
        let items = with(|s| s.blocked = Some(Blocked::Key));
        assert_eq!(
            items[2],
            action(
                "AI suggestions stopped: check your OpenRouter key…",
                Command::SetKey
            )
        );
    }

    #[test]
    fn fields_turned_off_are_listed_to_turn_back_on() {
        assert!(
            !labels(&menu(&status()))
                .iter()
                .any(|l| l.starts_with("Off in"))
        );
        let items = with(|s| s.off = vec!["Slack · Message #design".into()]);
        let list = submenu(&items, "Off in 1 field");
        assert!(list.contains(&action("Slack · Message #design", Command::TurnOn(0))));
        assert_eq!(
            list.last(),
            Some(&action("Turn all back on", Command::TurnAllOn))
        );
        let items = with(|s| s.off = vec!["a".into(), "b".into()]);
        submenu(&items, "Off in 2 fields");
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
    fn untrusted_offers_access_first() {
        let items = with(|s| s.trusted = false);
        assert_eq!(labels(&items)[1], "Grant Accessibility access…");
    }
}
