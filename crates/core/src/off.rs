//! The fields the writer asked for no suggestions in, from a card's "Don't
//! suggest in this field".
//!
//! A field is known by what the app says about it: the app, the site for a
//! web page, its id and its label (placeholder, description or title).
//! Either the id or the label is enough, as long as the app and site agree:
//! a web page may make up a new id on every load, and a label may be
//! reworded, but rarely both at once. A field with neither cannot be known
//! again; the platform keeps it off until Gramly quits instead.
//!
//! One field per line in `off.txt` beside the config, its parts separated
//! by tabs: app, site, id, label, and the name the menu shows.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Place {
    /// The app's bundle id (macOS) or desktop id (Linux).
    pub app: String,
    /// The page's host for a web field, empty otherwise.
    pub site: String,
    pub id: String,
    pub label: String,
    /// What the menu calls it: "Slack · Message #design".
    pub name: String,
}

impl Place {
    /// `who` is the app's name, or the site for a web field. The menu calls
    /// the field "Slack · Message #design", its label cut to 40 characters.
    pub fn new(app: String, site: String, id: String, label: String, who: &str) -> Self {
        let what: String = if label.is_empty() {
            "Text field".into()
        } else if label.chars().count() > 40 {
            format!("{}…", label.chars().take(39).collect::<String>())
        } else {
            label.clone()
        };
        let name = format!("{who} · {what}");
        Place {
            app,
            site,
            id,
            label,
            name,
        }
    }

    /// Whether it can be known again after a restart.
    pub fn lasting(&self) -> bool {
        !self.id.is_empty() || !self.label.is_empty()
    }

    /// Whether `other` is the same field.
    pub fn matches(&self, other: &Place) -> bool {
        self.app == other.app
            && self.site == other.site
            && ((!self.id.is_empty() && self.id == other.id)
                || (!self.label.is_empty() && self.label == other.label))
    }
}

pub fn path() -> PathBuf {
    crate::config::dir().join("off.txt")
}

pub fn load() -> Vec<Place> {
    load_from(&path())
}

/// A missing file is an empty list, and so is a line that does not read:
/// the worst a lost line does is bring suggestions back to one field.
pub fn load_from(path: &Path) -> Vec<Place> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut parts = line.split('\t').map(String::from);
            let place = Place {
                app: parts.next()?,
                site: parts.next()?,
                id: parts.next()?,
                label: parts.next()?,
                name: parts.next()?,
            };
            place.lasting().then_some(place)
        })
        .collect()
}

pub fn save(places: &[Place]) -> Result<()> {
    save_to(&path(), places)
}

pub fn save_to(path: &Path, places: &[Place]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A tab or line break in a label would split the line.
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut text = String::from("# Fields with no suggestions: app, site, id, label, name.\n");
    for p in places.iter().filter(|p| p.lasting()) {
        text.push_str(
            &[&p.app, &p.site, &p.id, &p.label, &p.name]
                .map(|s| clean(s))
                .join("\t"),
        );
        text.push('\n');
    }
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(id: &str, label: &str) -> Place {
        Place {
            app: "com.tinyspeck.slackmacgap".into(),
            site: String::new(),
            id: id.into(),
            label: label.into(),
            name: "Slack · Message #design".into(),
        }
    }

    #[test]
    fn the_menu_names_the_app_and_the_label() {
        let named =
            |label: &str| Place::new("app".into(), "".into(), "".into(), label.into(), "Slack");
        assert_eq!(named("Message #design").name, "Slack · Message #design");
        assert_eq!(named("").name, "Slack · Text field");
        let long = named(&"x".repeat(60)).name;
        assert_eq!(long.chars().count(), "Slack · ".chars().count() + 40);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn the_id_or_the_label_is_enough() {
        let off = place("composer", "Message #design");
        assert!(off.matches(&place("composer", "Message #general")));
        assert!(off.matches(&place(":r7:", "Message #design")));
        assert!(!off.matches(&place(":r7:", "Message #general")));
    }

    #[test]
    fn another_app_or_site_is_another_field() {
        let off = place("composer", "Message");
        let mut elsewhere = off.clone();
        elsewhere.site = "github.com".into();
        assert!(!off.matches(&elsewhere));
        elsewhere = off.clone();
        elsewhere.app = "com.apple.mail".into();
        assert!(!off.matches(&elsewhere));
    }

    #[test]
    fn a_field_with_nothing_to_know_it_by_never_matches() {
        let blank = place("", "");
        assert!(!blank.lasting());
        assert!(!blank.matches(&blank));
    }

    #[test]
    fn saved_and_loaded_unchanged() {
        let dir = std::env::temp_dir().join(format!("gramly-off-{}", std::process::id()));
        let path = dir.join("off.txt");
        let mut web = place("", "Leave a comment");
        web.site = "github.com".into();
        web.name = "github.com · Leave a\tcomment".into();
        let places = vec![place("composer", "Message #design"), web, place("", "")];
        save_to(&path, &places).unwrap();
        let back = load_from(&path);
        assert_eq!(back.len(), 2, "the field with no id or label is not kept");
        assert_eq!(back[0], places[0]);
        assert_eq!(back[1].name, "github.com · Leave a comment");
        let _ = std::fs::remove_dir_all(dir);
    }
}
