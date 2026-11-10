//! Where Squiggle never reads, on either platform. Grammarly and
//! LanguageTool publish no such list; each entry is a place whose text is
//! not prose, or is not for the model to see.

const TERMINAL: &str = "a terminal";
const PASSWORDS: &str = "a password manager";
const CODE: &str = "a code editor";

/// Apps by macOS bundle id or Hyprland class, in any case; an id ending in
/// `*` names every id it starts. Terminals: their text area is the
/// scrollback, and a shell command is never meant to be corrected. Password
/// managers: a secure note or a username is a plain text field, not a
/// password one, and what Squiggle reads is sent to the model; Grammarly
/// was caught reading Enpass. Code editors: what is typed is code, and the
/// commit message box in them is let go with it.
const APPS: &[(&str, &str)] = &[
    ("com.apple.Terminal", TERMINAL),
    ("com.googlecode.iterm2", TERMINAL),
    ("com.mitchellh.ghostty", TERMINAL),
    ("net.kovidgoyal.kitty", TERMINAL),
    ("org.alacritty", TERMINAL),
    ("com.github.wez.wezterm", TERMINAL),
    ("dev.warp.Warp-Stable", TERMINAL),
    ("co.zeit.hyper", TERMINAL),
    // Orca, an Electron terminal, stopped taking pasted images once woken.
    ("com.stablyai.orca", TERMINAL),
    ("kitty", TERMINAL),
    ("foot", TERMINAL),
    ("Alacritty", TERMINAL),
    ("org.wezfurlong.wezterm", TERMINAL),
    ("konsole", TERMINAL),
    ("gnome-terminal", TERMINAL),
    ("org.gnome.Ptyxis", TERMINAL),
    ("com.1password.1password", PASSWORDS),
    ("com.agilebits.onepassword7", PASSWORDS),
    ("com.bitwarden.desktop", PASSWORDS),
    ("org.keepassxc.keepassxc", PASSWORDS),
    ("in.sinew.Enpass-Desktop", PASSWORDS),
    ("me.proton.pass.electron", PASSWORDS),
    ("com.apple.Passwords", PASSWORDS),
    ("1Password", PASSWORDS),
    ("Bitwarden", PASSWORDS),
    ("keepassxc", PASSWORDS),
    ("Enpass", PASSWORDS),
    ("me.proton.Pass", PASSWORDS),
    ("Proton Pass", PASSWORDS),
    ("org.gnome.World.Secrets", PASSWORDS),
    ("org.gnome.seahorse.Application", PASSWORDS),
    ("com.microsoft.VSCode", CODE),
    ("com.microsoft.VSCodeInsiders", CODE),
    ("com.vscodium", CODE),
    ("com.todesktop.230313mzl4w4u92", CODE),
    ("com.exafunction.windsurf", CODE),
    ("dev.zed.Zed", CODE),
    ("dev.zed.Zed-Preview", CODE),
    ("com.apple.dt.Xcode", CODE),
    ("com.sublimetext.4", CODE),
    ("com.panic.Nova", CODE),
    ("com.jetbrains.*", CODE),
    ("com.google.android.studio", CODE),
    ("code", CODE),
    ("code-oss", CODE),
    ("codium", CODE),
    ("Cursor", CODE),
    ("Windsurf", CODE),
    ("sublime_text", CODE),
    ("jetbrains-*", CODE),
];

/// What the app `id` is, when Squiggle stays out of it.
pub fn app(id: &str) -> Option<&'static str> {
    let id = id.to_ascii_lowercase();
    APPS.iter()
        .find(|(listed, _)| {
            let listed = listed.to_ascii_lowercase();
            match listed.strip_suffix('*') {
                Some(start) => id.starts_with(start),
                None => id == listed,
            }
        })
        .map(|&(_, kind)| kind)
}

/// A field asks for one thing that is not a sentence: an address, a
/// number, a search. Zen's address bar is a `combobox`, Slack's "Find a
/// conversation" a `searchbox`.
const ROLES: &[&str] = &["combobox", "searchbox"];
const INPUTS: &[&str] = &["email", "url", "tel", "number", "search"];

/// The classes web code editors put on the element that takes the keys:
/// CodeMirror 6, Monaco (its textarea, then its EditContext) and Ace.
const EDITORS: &[&str] = &[
    "cm-content",
    "inputarea",
    "native-edit-context",
    "ace_text-input",
];

/// Why a field is not prose, from its ARIA role, its input type and its
/// classes as the page set them (any may be empty).
pub fn field(role: &str, input: &str, classes: &str) -> Option<&'static str> {
    if role.split_whitespace().any(|role| ROLES.contains(&role)) {
        Some("a search or address field")
    } else if INPUTS.contains(&input) {
        Some("an address or number field")
    } else if classes
        .split_whitespace()
        .any(|class| EDITORS.contains(&class))
    {
        Some("a code editor")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_password_managers_and_code_editors_are_left_alone() {
        assert_eq!(app("kitty"), Some(TERMINAL));
        assert_eq!(app("1Password"), Some(PASSWORDS));
        assert_eq!(app("com.1password.1password"), Some(PASSWORDS));
        assert_eq!(app("Cursor"), Some(CODE));
        assert_eq!(app("dev.zed.Zed"), Some(CODE));
    }

    #[test]
    fn an_app_is_matched_in_any_case_and_a_star_matches_a_family() {
        assert_eq!(app("alacritty"), Some(TERMINAL));
        assert_eq!(app("com.jetbrains.intellij"), Some(CODE));
        assert_eq!(app("jetbrains-pycharm"), Some(CODE));
    }

    #[test]
    fn other_apps_and_near_names_are_read() {
        assert_eq!(app("zen"), None);
        assert_eq!(app("com.slack.Slack"), None);
        assert_eq!(app("kitty-notes"), None);
        assert_eq!(app("vscode-notes"), None);
    }

    #[test]
    fn search_address_and_number_fields_are_not_prose() {
        assert!(field("combobox", "", "").is_some());
        assert!(field("searchbox", "search", "").is_some());
        assert!(field("textbox", "email", "").is_some());
        assert!(field("", "url", "").is_some());
        assert!(field("", "tel", "").is_some());
        assert!(field("", "number", "").is_some());
    }

    #[test]
    fn a_web_code_editor_is_found_by_its_class() {
        assert!(field("textbox", "", "cm-content cm-lineWrapping").is_some());
        assert!(field("", "", "inputarea monaco-mouse-cursor-text").is_some());
        assert!(field("", "", "ace_text-input").is_some());
    }

    #[test]
    fn a_message_box_is_prose() {
        assert_eq!(field("textbox", "", "ql-editor"), None);
        assert_eq!(field("textbox", "text", ""), None);
        assert_eq!(field("", "", "xcm-content"), None);
        assert_eq!(field("", "", ""), None);
    }
}
