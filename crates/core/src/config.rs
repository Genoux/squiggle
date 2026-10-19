//! Two layers: these defaults, then `~/.config/gramly/config.toml`.
//!
//! Flat `key = value`, the same shape as Flow's file, so one person editing
//! both never has to remember which is which. An absent file is normal; a
//! broken one is an error, because silently ignoring a line the user wrote
//! is a setting that looks applied and is not.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Master switch. Off draws nothing and sends nothing.
    pub enabled: bool,
    /// Whether text leaves the machine for the model's pass. Off keeps the
    /// local pass, which needs no key.
    pub ai: bool,
    /// Never log this value.
    pub openrouter_key: Option<String>,
    pub dialect: String,
    /// Bundle ids of the apps turned off from the menu: nothing is read,
    /// drawn or sent while one of them is in front.
    pub off_in: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            ai: true,
            openrouter_key: None,
            dialect: "american".into(),
            off_in: Vec::new(),
        }
    }
}

pub fn dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("gramly");
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/gramly")
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
                "enabled" => config.enabled = boolean(&at, &value)?,
                "ai" => config.ai = boolean(&at, &value)?,
                "openrouter_key" => config.openrouter_key = Some(value).filter(|v| !v.is_empty()),
                "dialect" => match value.as_str() {
                    "american" | "british" | "canadian" | "australian" | "indian" => {
                        config.dialect = value
                    }
                    other => bail!("{at}: unknown dialect {other:?}"),
                },
                "off_in" => {
                    config.off_in = value
                        .split(',')
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(String::from)
                        .collect()
                }
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
        let mut text = format!(
            "enabled = {}\nai = {}\ndialect = \"{}\"\n",
            self.enabled, self.ai, self.dialect
        );
        if !self.off_in.is_empty() {
            text.push_str(&format!("off_in = \"{}\"\n", self.off_in.join(", ")));
        }
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
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", word.trim())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gramly-config-{}-{name}", std::process::id()));
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
            enabled: true,
            ai: false,
            openrouter_key: Some("sk-or-v1-abc".into()),
            dialect: "british".into(),
            off_in: vec!["com.tinyspeck.slackmacgap".into(), "com.apple.mail".into()],
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
}
