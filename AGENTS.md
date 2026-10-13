# AGENTS.md

Notes on why parts of Gramly are built the way they are. Same rule as Flow's:
record what was measured or debugged, and the mistake each decision guards.

Gramly is a grammar checker that draws on top of other apps' text fields.
macOS ships first; Linux (Wayland, Hyprland) follows on the same engine.

## Layout

- `crates/core` (`gramly-core`) knows text and nothing about screens: Harper,
  the model, merging, config. CI tests it on Linux as well as macOS so no
  platform assumption creeps in before the Linux surface exists.
- `src/macos` is the surface: Accessibility in (`ax.rs`), panels out
  (`overlay.rs`), the run loop and menu (`app.rs`), the paste fallback
  (`paste.rs`), Flow's palette (`theme.rs`).
- One binary. Flow splits daemon and console because iced brings wgpu; Gramly
  draws with AppKit directly and has no settings window yet. If one is added
  with iced, it goes in its own workspace root like Flow's console.

## Checking

- Two passes, two threads (`worker.rs`). Harper answers in milliseconds and
  draws first; the model answers in about a second and arrives as a second
  outcome for the same generation. Underlines never wait on the network.
- The model returns edits, never a rewritten paragraph: a rewrite has to be
  diffed back into edits to be underlined, and the diff of a paraphrase is one
  edit nobody can accept a word of. Edits quote their words (`original`, plus
  up to three words `before`) instead of giving offsets - models quote
  reliably and count characters badly. `ai::locate` finds them; an edit whose
  words are not in the text is dropped, never guessed onto the nearest match.
- Edits over 80 chars are refused as rewrites.
- Where Harper and the model mark the same words, the model wins
  (`fix::merge`): Harper finds misspellings exactly but picks the replacement
  from the dictionary alone.
- The model is asked per paragraph, cached by exact paragraph text. Typing in
  one paragraph of a long email costs one request; refocusing costs none.
- Pinned to `anthropic/claude-haiku-5.5` on the `anthropic` provider with
  fallbacks off, the same discipline as Flow's refiner: a reroute is a
  different machine. Chosen because it is cheaper than Gemini 3.1 Flash-Lite
  on OpenRouter ($0.10/$0.50 per M at 2026-10-12) and follows "do not change
  the voice" well. Not yet measured against alternatives - do that before
  swapping it.
- Config and dictionary are read per check, as Flow reads its style per
  dictation, so a setting applies on the next pause rather than the next launch.
- The key never reaches curl's argv; same `-K -` and 0600 body file as Flow.

## Offsets

- `Fix` offsets are chars. macOS Accessibility counts UTF-16; `Fix::utf16` is
  the only conversion. An emoji before a typo is the test case.

## macOS surface

- Polled at 100ms, not driven by `AXObserver`. Notifications are per process
  and per element, several Electron apps never send `AXValueChanged`, and
  observers must be re-registered whenever focus moves.
- Every AX call is IPC into another app; a 150ms messaging timeout keeps a
  hung app from hanging Gramly.
- Panels are `NonactivatingPanel`, `hidesOnDeactivate = false`. Clicking
  Accept must not steal focus from the field being corrected, and an accessory
  app is never active, so a panel that hides on deactivation never shows.
- The underline panel ignores the mouse, so clicks reach the text. Hover is
  detected by polling the pointer position against the word rectangles.
- Writing a fix: select the range, set `AXSelectedText`, then **read the value
  back**. Chromium and Electron accept the write and do nothing. When the read
  disagrees, the fix is pasted over the selection and the clipboard restored
  after 250ms.
- Accept all applies back to front so each write leaves the remaining offsets
  valid. If the app refuses direct writes, the whole corrected text is pasted
  once over a select-all - a dozen pastes would race the clipboard. That
  fallback loses rich formatting in that field; a known trade-off.
- Terminals are skipped by bundle id, password fields by role. Both are
  deliberate and both are what Grammarly does.
- The Accessibility grant is tied to the code signature. Install signs ad hoc,
  which changes every build, so every update needs the grant again. A stable
  self-signed identity would fix that; not done yet.

## Repository

- The git history carries no agent attribution: no `Co-authored-by:` trailers,
  no tool signatures in messages, no agent branches. Agent tooling directories
  stay untracked. Same rule as Flow.
- Versions: root and `crates/core` move together, and the release workflow
  refuses a tag that disagrees with either. Bump with a `chore: bump version
  to X.Y.Z` commit, then tag `vX.Y.Z`. Releases are drafts until someone has
  run the build.
