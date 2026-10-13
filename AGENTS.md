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
- The model returns the paragraph corrected; `diff.rs` turns the difference
  into fixes. Asked for a list of edits instead, Haiku under-reported: it
  fixed "i went" and left "paris", "monday" and a missing "?" on the same
  line, every run. Correcting the text is the task it does completely.
- `diff` is a word-level LCS (words, whitespace runs, single punctuation
  marks), line by line so no fix ever crosses or eats a line break, merging
  changes separated only by whitespace ("jpense" → "je pense que" is one
  fix). A pure insertion takes the word before it ("think" → "think?").
  A correction touching over 60% of a paragraph's words is a rewrite and is
  dropped whole; a single fix over 80 chars is dropped.
- A paragraph is sent with its line break when it has one. Without it every
  paragraph looked unfinished and the model never added end punctuation;
  the prompt says which case it is.
- Titles ("Capitalization", "Majuscule", ...) come from the kind of change,
  in the language of the sentence it is in.
- Harper draws first and is the offline fallback; once the model has
  answered for the current text, its fixes are the whole answer. Harper on
  short French slang offered "Eat" for "ERT", and only runs on sentences
  detected as clearly English.
- The model is asked per paragraph, cached by exact paragraph text. Typing in
  one paragraph of a long email costs one request; refocusing costs none.
- Pinned to `anthropic/claude-haiku-5.5` on the `anthropic` provider with
  fallbacks off, the same discipline as Flow's refiner: a reroute is a
  different machine. Chosen because it is cheaper than Gemini 3.1 Flash-Lite
  on OpenRouter ($0.10/$0.50 per M at 2026-10-12) and follows "do not change
  the voice" well. Not yet measured against alternatives - do that before
  swapping it.
- Language is detected per paragraph, English vs French only
  (`lang.rs`): among all of whatlang's languages a plain ten-word English
  sentence came back unreliable; between two it is decisive. Harper is
  English-only and runs on English paragraphs alone - on French it flags
  every word. The model is told the language by name, as Flow does.
- Config and dictionary are read per check, as Flow reads its style per
  dictation, so a setting applies on the next pause rather than the next launch.
- The key never reaches curl's argv; same `-K -` and 0600 body file as Flow.

## Offsets

- `Fix` offsets are chars. macOS Accessibility counts UTF-16; `Fix::utf16` is
  the only conversion. An emoji before a typo is the test case.

## macOS surface

- Polled at 50ms, not driven by `AXObserver`. 100ms made the card feel
  sluggish next to Grammarly's, since hover is a poll too. Notifications are per process
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
- The Accessibility grant is tied to the signature's designated requirement.
  Ad hoc, that requirement is the build's hash, so every install lost the
  grant. Install now signs with a "Gramly Local Signing" certificate it makes
  once in the login keychain; the requirement names the certificate and the
  grant survives rebuilds.
- Nothing animates. Core Animation's implicit actions drew every new
  underline sliding in from the panel corner; layer changes go through
  `overlay::still` with actions disabled, panels have no window animation.
- Underlines are carried across edits (`fix::rebase`) instead of cleared on
  every keystroke, which flickered: a fix the edit touched is dropped, the
  rest shift. Letters typed onto a word's edge count as touching it; a space
  or punctuation does not.
- Two cards. One fix in a sentence gets the small one (title, the
  replacement to click, Dismiss). Two or more get the sentence card with
  every change inline and "Correct the sentence". Grouping is
  `fix::sentence`, which splits early on abbreviations - a smaller group,
  never a wrong one.

## Repository

- The git history carries no agent attribution: no `Co-authored-by:` trailers,
  no tool signatures in messages, no agent branches. Agent tooling directories
  stay untracked. Same rule as Flow.
- Versions: root and `crates/core` move together, and the release workflow
  refuses a tag that disagrees with either. Bump with a `chore: bump version
  to X.Y.Z` commit, then tag `vX.Y.Z`. Releases are drafts until someone has
  run the build.
