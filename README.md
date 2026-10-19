# Gramly

Write anywhere; mistakes get underlined where you typed them. Hover one to see
the fix, click **Accept**, or take them all at once.

Gramly is a grammar checker for the text field you already have open - Mail,
Notes, Slack, a browser, anything that tells macOS what it is showing. Spelling
and grammar are checked on your Mac by [Harper](https://github.com/Automattic/harper);
the subtler mistakes go to Gemini 3.5 Flash-Lite through your own
[OpenRouter](https://openrouter.ai) key, for about a dollar a month of heavy use.

## Requirements

| | |
|---|---|
| macOS | 13 or later, Apple silicon |
| Access | Accessibility, granted once in System Settings |
| Account | An OpenRouter key, only for AI suggestions |

Linux is supported on Hyprland only; see [Linux](#linux).

## Install

```bash
git clone https://github.com/Genoux/gramly
cd gramly && ./packaging/install.sh
```

Or download a release tarball, unpack it, and run the same
`packaging/install.sh` from inside - it uses the binary it finds there instead
of building one.

That puts **Gramly.app** in `~/Applications`, starts it, and starts it again at
every login. Nothing is written outside your home directory and nothing needs
`sudo`.

The first launch asks for Accessibility access: that is how Gramly reads the
field you are typing in and writes a fix back. Then open the menu bar icon →
**OpenRouter key…** for AI suggestions, or skip it and keep the on-device
checks only.

Updating is `git pull && ./packaging/install.sh`. Removing it is
`./packaging/uninstall.sh`, which leaves your settings and dictionary alone.

## Linux

Hyprland on Wayland, and only Hyprland: Wayland tells an app nothing about
where other windows or the pointer are, and Hyprland's socket does. On
another compositor Gramly refuses to install rather than underline nothing.

| | |
|---|---|
| Session | Hyprland, with a tray that speaks StatusNotifierItem (Waybar, Quickshell, ...) |
| Build | Rust, GTK 4 and `gtk4-layer-shell` (`pacman -S gtk4 gtk4-layer-shell`) |

```bash
git clone https://github.com/Genoux/gramly
cd gramly && ./packaging/install.sh
```

That installs `~/.local/bin/gramly` and a systemd user unit that starts with
your graphical session; nothing needs `sudo`. The unit relies on the session
having `WAYLAND_DISPLAY` and `HYPRLAND_INSTANCE_SIGNATURE` in systemd's user
environment (uwsm does that); the installer checks and tells you if not.
Removing it is `./packaging/uninstall.sh`.

**Chrome and Electron apps need a flag.** They build no accessibility tree
unless started with `--force-renderer-accessibility`, so without it Gramly
sees no text field in them. The installer offers to add it to
`~/.config/chrome-flags.conf` and `~/.config/electron-flags.conf`, which
Chrome and the Arch `electron` packages read, and restart the app afterwards.
Slack, ChatGPT, Orca and other apps that bundle their own Electron read no
flag file; add the flag to the `Exec` line of a copy of their `.desktop`
file in `~/.local/share/applications`.

`gramly doctor` shows what Gramly can see in the field you click into, and
the tray icon has the same menu as the Mac's, except that **Personal
dictionary…** opens `dictionary.txt` in your text editor.

## Daily use

| | |
|---|---|
| Red underline | A mistake - spelling, grammar, a wrong word |
| Blue underline | Correct, but could read better |
| Hover an underline | Your sentence with the fix in place, then **Accept** / **Dismiss** / **Add to dictionary** |
| Menu bar icon (tray icon on Linux) | Pause for an hour, turn off in the app in front, AI on/off, key, spelling, personal dictionary |
| `gramly doctor` | What Gramly can see in the field you click into |
| `gramly check < file.txt` | The same suggestions, printed |
| `gramly check --improve < file.txt` | The clearer wording the app proposes, printed |

Terminals are skipped on purpose, as are password fields.

## Configuration

`~/.config/gramly/config.toml`, optional, and written by the menu for you:

```toml
enabled = true
ai = true
dialect = "american"   # british, canadian, australian, indian
off_in = "com.tinyspeck.slackmacgap"   # apps turned off from the menu
openrouter_key = "sk-or-..."
```

Your dictionary opens from the menu bar icon → **Personal dictionary…**, and
lives in `~/.config/gramly/dictionary.txt`, one word per line. A word
in it is never flagged and never changed, by the on-device pass or the AI.

## Privacy

The on-device pass never leaves the Mac. With AI suggestions on, the paragraph
you are editing is sent to OpenRouter, which routes it to Anthropic; nothing is
stored by Gramly. Password fields are never read.

## Developing

```bash
./dev            # run from this checkout, restart on save
cargo test --workspace
GRAMLY_OPENROUTER_KEY=sk-or-... cargo test -p gramly-core -- --ignored   # live model
cargo run -- lab          # accept fixes in real text fields, off screen, and audit them
cargo run -- preview      # draw every card and the dictionary to target/preview, light and dark
./sandbox                 # every release check, with a report; --ai, --models, --screen, --all
```

main takes no direct commits: work on a branch and open a pull request,
which merges once CI passes. Once per clone, `git config core.hooksPath
.githooks` makes git refuse a commit on main before GitHub refuses the push.

See [AGENTS.md](AGENTS.md) for why things are built the way they are,
[DESIGN.md](DESIGN.md) for how the card and the dictionary look (the full
design system is `design/index.html`, its values `design/tokens.json`), and
[TROUBLESHOOTING.md](TROUBLESHOOTING.md) when something does not underline.

The bundled font, Atkinson Hyperlegible Next, is under the SIL Open Font
License ([assets/fonts/OFL.txt](assets/fonts/OFL.txt)).
