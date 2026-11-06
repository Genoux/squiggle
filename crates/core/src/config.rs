//! Two layers: these defaults, then `~/.config/squiggle/config.toml`.
//!
//! Flat `key = value`, the same shape as Flow's file, so one person editing
//! both never has to remember which is which. An absent file is normal; a
//! broken one is an error, because silently ignoring a line the user wrote
//! is a setting that looks applied and is not.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Whether text leaves the machine for the model's pass. Off keeps the
    /// local pass, which needs no key.
    pub ai: bool,
    /// Never log this value.
    pub openrouter_key: Option<String>,
    pub dialect: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ai: true,
            openrouter_key: None,
            dialect: "american".into(),
        }
    }
}

pub fn dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("squiggle");
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/squiggle")
}

/// Squiggle was called Gramly before 1.0 and kept its settings, dictionary
/// and fields turned off in a `gramly` folder beside this one. Moved once,
/// so an update keeps them; never onto a folder that already exists, which
/// would leave two sets to merge.
pub fn adopt_old_dir() {
    let new = dir();
    if let Some(old) = new.parent().map(|parent| parent.join("gramly"))
        && let Err(e) = adopt(&old, &new)
    {
        eprintln!("could not move {} to {}: {e}", old.display(), new.display());
    }
}

fn adopt(old: &Path, new: &Path) -> std::io::Result<()> {
    if old.is_dir() && !new.exists() {
        std::fs::rename(old, new)?;
    }
    Ok(())
}

pub fn config_path() -> PathBuf {
    dir().join("config.toml")
}

/// One word per line; `#` starts a comment.
pub fn dictionary_path() -> PathBuf {
    dir().join("dictionary.txt")
}

impl Config {
    pub fn load() -> Result<Self> {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let mut config = Self::default();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(config),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = format!("{}:{}", path.display(), number + 1);
            let Some((key, value)) = line.split_once('=') else {
                bail!("{at}: expected `key = value`, found {line:?}");
            };
            let value = value.trim().trim_matches('"').to_string();
            match key.trim() {
                "ai" => config.ai = boolean(&at, &value)?,
                "openrouter_key" => config.openrouter_key = Some(value).filter(|v| !v.is_empty()),
                "dialect" => match value.as_str() {
                    "american" | "british" | "canadian" | "australian" | "indian" => {
                        config.dialect = value
                    }
                    other => bail!("{at}: unknown dialect {other:?}"),
                },
                // Settings since removed: "Check as I type" and apps turned
                // off from the menu. Read and let go, so the file still
                // loads and an old `enabled = false` cannot leave Squiggle
                // off with no switch to turn it on; the next save drops them.
                "enabled" | "off_in" => {}
                // Named, not echoed: the line may be a key pasted in the wrong place.
                other => bail!("{at}: unknown setting {other:?}"),
            }
        }
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    /// Loads, changes and saves in one step; a file that cannot be read
    /// starts from the defaults rather than blocking the change.
    pub fn update(change: impl FnOnce(&mut Config)) -> Result<()> {
        let mut config = Self::load().unwrap_or_default();
        change(&mut config);
        config.save()
    }

    /// Written to a temporary file and renamed over the target, with the key
    /// readable by this user only.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut text = format!("ai = {}\ndialect = \"{}\"\n", self.ai, self.dialect);
        if let Some(key) = &self.openrouter_key {
            text.push_str(&format!("openrouter_key = \"{key}\"\n"));
        }
        let staged = path.with_extension("toml.new");
        let _ = std::fs::remove_file(&staged);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&staged)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&staged, path)?;
        Ok(())
    }

    pub fn harper_dialect(&self) -> harper_core::Dialect {
        use harper_core::Dialect;
        match self.dialect.as_str() {
            "british" => Dialect::British,
            "canadian" => Dialect::Canadian,
            "australian" => Dialect::Australian,
            "indian" => Dialect::Indian,
            _ => Dialect::American,
        }
    }
}

fn boolean(at: &str, value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => bail!("{at}: expected true or false, found {other:?}"),
    }
}

pub fn dictionary() -> Vec<String> {
    std::fs::read_to_string(dictionary_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

/// Replaces the word list, keeping the comment lines at the top of the
/// file. Staged and renamed, so a crash never leaves half a list.
pub fn save_dictionary(words: &[String]) -> Result<()> {
    let path = dictionary_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut text: String = old
        .lines()
        .take_while(|l| l.trim().starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect();
    for word in words {
        text.push_str(word.trim());
        text.push('\n');
    }
    let staged = path.with_extension("txt.new");
    std::fs::write(&staged, text)?;
    std::fs::rename(&staged, &path)?;
    Ok(())
}

pub fn add_to_dictionary(word: &str) -> Result<()> {
    use std::io::Write;
    let path = dictionary_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let ends_unfinished =
        std::fs::read(&path).is_ok_and(|bytes| bytes.last().is_some_and(|last| *last != b'\n'));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(appended(ends_unfinished, word).as_bytes())?;
    Ok(())
}

/// A file edited by hand may not end in a newline; appended straight on,
/// the new word joined the last one ("foo" + "bar" became "foobar").
fn appended(ends_unfinished: bool, word: &str) -> String {
    let lead = if ends_unfinished { "\n" } else { "" };
    format!("{lead}{}\n", word.trim())
}

/// The words as a window lists them: by name, one entry for each spelling
/// that differs only in case.
pub fn sorted_dictionary() -> Vec<String> {
    listed(dictionary())
}

fn listed(mut words: Vec<String>) -> Vec<String> {
    words.sort_by_key(|word| word.to_lowercase());
    words.dedup_by_key(|word| word.to_lowercase());
    words
}

/// What typing `input` into the dictionary's field adds: the word, unless
/// it is empty, has whitespace inside, or is already in `known`.
fn word_to_add(input: &str, known: &[String]) -> Option<String> {
    let word = input.trim();
    let new = !word.is_empty()
        && !word.contains(char::is_whitespace)
        && !known
            .iter()
            .any(|w| w.to_lowercase() == word.to_lowercase());
    new.then(|| word.to_string())
}

/// Adds what was typed into the dictionary's field. Whether the list changed.
pub fn learn_word(input: &str) -> Result<bool> {
    let Some(word) = word_to_add(input, &dictionary()) else {
        return Ok(false);
    };
    add_to_dictionary(&word)?;
    Ok(true)
}

/// Takes `word` out of the list, in every case, since the window lists the
/// spellings as one entry. The rest stays as written: saving the sorted,
/// deduplicated list dropped "zen" when an unrelated word was removed
/// beside "Zen". Whether it was there.
pub fn forget_word(word: &str) -> Result<bool> {
    let words = dictionary();
    let kept = without(&words, word);
    let found = kept.len() != words.len();
    if found {
        save_dictionary(&kept)?;
    }
    Ok(found)
}

fn without(words: &[String], word: &str) -> Vec<String> {
    let word = word.to_lowercase();
    words
        .iter()
        .filter(|w| w.to_lowercase() != word)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_folder_moves_once_and_never_over_a_new_one() {
        let root = std::env::temp_dir().join(format!("squiggle-adopt-{}", std::process::id()));
        let (old, new) = (root.join("gramly"), root.join("squiggle"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("dictionary.txt"), "Hyprland\n").unwrap();
        adopt(&old, &new).unwrap();
        assert!(!old.exists());
        assert_eq!(
            std::fs::read_to_string(new.join("dictionary.txt")).unwrap(),
            "Hyprland\n"
        );

        std::fs::create_dir_all(&old).unwrap();
        adopt(&old, &new).unwrap();
        assert!(
            old.exists(),
            "a second folder is left for the user to merge"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_word_appended_to_an_unfinished_line_starts_its_own() {
        assert_eq!(appended(true, " bar "), "\nbar\n");
        assert_eq!(appended(false, "bar"), "bar\n");
    }

    #[test]
    fn removing_a_word_takes_its_spellings_and_leaves_the_rest_as_written() {
        let words = ["zen", "Hyprland", "Zen", "apple", "hyprland"].map(String::from);
        assert_eq!(without(&words, "Hyprland"), ["zen", "Zen", "apple"]);
    }

    #[test]
    fn the_list_is_sorted_by_name_with_one_entry_per_spelling() {
        let words = ["zen", "Hyprland", "apple", "hyprland"].map(String::from);
        assert_eq!(listed(words.to_vec()), ["apple", "Hyprland", "zen"]);
    }

    #[test]
    fn a_typed_word_is_trimmed() {
        assert_eq!(word_to_add("  Hyprland \n", &[]), Some("Hyprland".into()));
    }

    #[test]
    fn empty_or_spaced_input_adds_nothing() {
        assert_eq!(word_to_add("", &[]), None);
        assert_eq!(word_to_add("   ", &[]), None);
        assert_eq!(word_to_add("two words", &[]), None);
    }

    #[test]
    fn a_known_word_is_not_added_again_whatever_its_case() {
        assert_eq!(word_to_add("hyprland", &["Hyprland".into()]), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("squiggle-config-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    #[test]
    fn absent_file_is_the_defaults() {
        let path = scratch("absent");
        assert_eq!(Config::load_from(&path).unwrap(), Config::default());
    }

    #[test]
    fn round_trips_and_stays_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch("roundtrip");
        let config = Config {
            ai: false,
            openrouter_key: Some("sk-or-v1-abc".into()),
            dialect: "british".into(),
        };
        config.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), config);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn an_unknown_line_is_an_error_not_a_shrug() {
        let path = scratch("unknown");
        std::fs::write(&path, "ai = true\nsk-or-v1-pasted-here\n").unwrap();
        let err = Config::load_from(&path).unwrap_err().to_string();
        assert!(err.contains(":2:"), "{err}");
    }

    #[test]
    fn a_removed_setting_still_loads_and_is_dropped_on_save() {
        let path = scratch("removed");
        let old = "enabled = false\nai = false\noff_in = \"com.apple.mail\"\n";
        std::fs::write(&path, old).unwrap();
        let config = Config::load_from(&path).unwrap();
        assert!(!config.ai);
        config.save_to(&path).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            !saved.contains("off_in") && !saved.contains("enabled"),
            "{saved}"
        );
    }
}
