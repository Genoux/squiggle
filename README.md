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

Linux (Wayland, Hyprland) is next; the engine already builds and is tested there.

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

## Daily use

| | |
|---|---|
| Red underline | A mistake - spelling, grammar, a wrong word |
| Blue underline | Correct, but could read better |
| Hover an underline | The fix, why, **Accept** / **Dismiss** / **Add to dictionary** |
| Menu bar icon | Pause for an hour, turn off in the app in front, AI on/off, key, spelling, personal dictionary |
| `gramly doctor` | What Gramly can see in the field you click into |
| `gramly check < file.txt` | The same suggestions, printed |

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
./sandbox                 # every release check, with a report; --ai, --models, --screen, --all
```

See [AGENTS.md](AGENTS.md) for why things are built the way they are, and
[TROUBLESHOOTING.md](TROUBLESHOOTING.md) when something does not underline.
