# AGENTS.md

Notes on why parts of Squiggle are built the way they are. Same rule as Flow's:
record what was measured or debugged, and the mistake each decision guards.

Squiggle is a grammar checker that draws on top of other apps' text fields.
macOS ships first; Linux (Wayland, Hyprland) follows on the same engine.

## Layout

- `crates/core` (`squiggle-core`) knows text and nothing about screens: Harper,
  the model, merging, config. CI tests it on Linux as well as macOS so no
  platform assumption creeps in before the Linux surface exists.
- `src/surface` is everything between text and screen that is not a
  platform: the run loop (`engine/`), what a card says (`card.rs`), where it
  goes and when the pointer has left it (`geometry.rs`, y down from the
  top-left as Accessibility and Hyprland report), writing a fix into a field
  (`write.rs`) and every Ink value, colour and measure (`theme.rs`). A platform gives
  it a `Host`, a `Field` and an `Overlay` and nothing else; logic that
  exists in both is a bug.
- `src/macos` is the first platform: Accessibility in (`ax.rs`), panels out
  (`overlay.rs`, which flips to Cocoa's y-up at its own edge), the menu
  (`app.rs`), the OS handle (`host.rs`), the paste fallback (`paste.rs`),
  the Ink palette's AppKit half and the font (`theme.rs`), and `squiggle
  preview` (`preview.rs`).
- `src/linux` is the Hyprland surface: AT-SPI in (`atspi.rs`), Hyprland's
  IPC for where windows are (`hypr.rs`). Hyprland only: Wayland tells no
  client where a window or the pointer is, and Hyprland's socket does;
  GNOME has no layer-shell to draw an overlay with at all. It draws Ink
  from `surface/theme.rs`: its stylesheet (`card::css`) is built from those
  values, so there is no second copy to drift.
- `design/` is the design system: `index.html` shows every token and
  component, `tokens.json` holds the values for the code and for the Linux
  surface. It lives in the repo because the repo is private and the page must
  change with the code; tests fail when either disagrees with the code.
- One binary. Flow splits daemon and console because iced brings wgpu; Squiggle
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
  fix). A pure insertion takes the word before it ("think" → "think?"),
  and so do spaces turned into punctuation ("yes  was" → "yes, was"
  shows "yes,"): left on the spaces, the card said "Remove the extra
  space" and hid the comma. Only a change that is spaces both sides is
  shown as spacing. The spaces ending a line are never part of a fix:
  "now? " came back as "? " -> "", past the end-mark rule, and accepting
  it in a web field did nothing.
  A correction touching over 60% of a paragraph's words is a rewrite and is
  dropped whole; a single fix over 80 chars is dropped.
- A wrong real word is corrected to the word it looks like, never to a
  phrase that fits: asked only to correct, Haiku fixed "came we some
  restructure" to "came out of" one run in three; told that a wrong word
  is a slip for a word that looks like it, "with" in every run. Measured
  three runs each over a dozen real messages with `squiggle check`, which
  prints what the app draws (the model's fixes, protected text removed).
  The prompt also names agreement after "there", a comma joining two
  sentences, and shorthand never expanded ("u", "ok"): each was a miss
  or a wrong suggestion in the benchmark. Shorthand is kept only where it
  reads as shorthand: "for nw?" was left as "nw" (no worries) every run,
  and no underline appeared. With the rule and two examples, "now" five
  runs in five, "nw, take your time" still left, benchmark up (F0.97,
  94% exact). Temperature 0 changed nothing measurable;
  reasoning off missed typos ("Pickan", "oldad") and still varied;
  reasoning medium took 6s a paragraph against 2.2s, past `MODEL_LATE`.
  Reasoning stays low.
- The model's fixes get rules it only keeps most of the time
  (`diff::final_form`): a fix to a sentence's first word comes with its
  capital, and a fix that only turns "!" into another end mark is
  dropped. Asked about "thi is th teste for me!" it answered "this" two
  runs in three; accepted, "this" came back as "This", a fix on top of a
  fix, where Grammarly offers "This" once. On a finished line it offered
  "!" -> ".". Benchmark unchanged (F0.95, 92% exact) with both cases added.
  A third: an ellipsis is never added. On a line still being typed it
  turned "congelationÉ" into "congelation..." two runs in three. A
  fourth: an end mark the writer typed is never taken away. After an
  accepted improvement, "Can you think about conservation for now?" was
  offered without its "?" one run in three.
- A question ended with a period ("Can you think about it.") is left
  alone. Told to fix it, with an example of that very shape, the model
  still kept the period every run; Grammarly's free check leaves it too
  and only its paid rewrite touched it. A polite request may end in one.
- A paragraph is sent with its line break when it has one. Without it every
  paragraph looked unfinished and the model never added end punctuation;
  the prompt says which case it is.
- Titles are generic, as Grammarly's are ("Correct your spelling",
  "Correct your punctuation"): the change explains itself, and titles naming
  the rule ("Majuscule et ponctuation") read as a lecture.
- With the model on, nothing of Harper's is drawn: the model's answer is
  the whole answer, a second or two after the pause. Harper's unknown
  words were once drawn beside it, at once, and offered "tmrw" -> "thaw"
  and "oldad" -> "olds" (then switched to the model's "old ad" in the open
  card). Drawing Harper first and the model second made suggestions
  appear and then vanish, often while hovered (the log showed "card's fix
  withdrawn" each time). Harper draws when the model is off, failing, or
  has not answered within `MODEL_LATE` (4s). A failed model check sends only
  its failure: the worker once followed it with an empty answer, the app
  took that as the model finding nothing, and with the key out of credits
  not even Harper's underlines were drawn.
- Harper and the model, when both are drawn: once the model has
  answered for the current text, its fixes are the whole answer. Harper on
  short French slang offered "Eat" for "ERT", and only runs on sentences
  detected as clearly English.
- Clearer wording is a third pass (`improve.rs`), its own request on its
  own thread, so the corrections' prompt and benchmark are untouched. The
  model names each awkward sentence and its rewrite; the fix is cut to the
  words that change, so accept, rebase and dismiss work as for corrections.
  It is drawn blue, under its first changed words (a rewrite spans most of
  a sentence and may wrap), only once the corrections have answered, and
  only in a sentence with no correction left - the two cards would fight
  over the words. Its card is never part of "Correct the sentence".
  Dropped in code: a rewrite that only respells or repunctuates, one
  keeping under 40% of the words (a polite French request came back as
  "Tu pourrais venir demain ?"), and the sentence still being typed.
  `examples/bench_improve.rs` over `bench/improve.json`, 2026-10-14, on
  the corrections' route: no proposal on 36 runs of text that reads fine,
  29/30 on awkward text, p95 1.9s, $0.15 per 1000. It has no wording
  score; read `--show` before changing the prompt.
- The model is asked per paragraph, cached by exact paragraph text. Typing in
  one paragraph of a long email costs one request; refocusing costs none.
- Pinned to `google/gemini-3.5-flash-lite` on `google-ai-studio`,
  reasoning low, fallbacks off, the same discipline as Flow's refiner: a
  reroute is a different machine. Chosen by the benchmark
  (`cargo run --release -p squiggle-core --example bench`): every route in
  `crates/core/bench/routes.json` against the cases in `bench/cases.json`,
  through the app's own prompt, diff and protection, scored on F0.5 (a
  wrong suggestion costs more trust than a missed one), exact results,
  same answer across runs, latency and cost. 2026-10-13, 60 cases x 3
  runs, three passes: Flash-Lite F0.95-0.97, precision 0.96-0.98, p95
  1.2-1.5s, $0.20 per 1000 checks; Haiku 5.5 (the model before) F0.96,
  precision 0.95-0.96, p95 3.2-3.7s, $0.11. Underlines arriving late was
  the complaint, so the faster one won the tie. Sonnet 5.5 and Gemini 3.8
  Flash scored no better and cost 7-17x; GPT-6 Luna, DeepSeek v4.1 Flash,
  Nemotron 3.5 Lightning and Solar Mini 4 were near precision 0.68; MiMo,
  GLM 5.3 Flash and Qwen 3.8 Flash failed calls; Mercury 2.5 0.79. The
  prompt was tuned on Haiku first, which favours it. Run the benchmark
  before changing the model or the prompt, and add a case for every bad
  suggestion a user reports.
- Language is detected per paragraph, English vs French only
  (`lang.rs`): among all of whatlang's languages a plain ten-word English
  sentence came back unreliable; between two it is decisive. Harper is
  English-only and runs on English paragraphs alone - on French it flags
  every word. The model is told the language by name, as Flow does.
  A paragraph is named one language only if no sentence of four words or
  more is uncertain. "Can you think about conservation for now?" scored
  0.33 English and was skipped, the paragraph went out as French, and the
  model dropped the English sentence (a rewrite, so nothing was shown) or
  mangled the French. Counted as unknown, the paragraph gets the mixed
  prompt and "Est a tu vue la lune." comes back right most runs; Haiku
  and Flash-Lite still miss it one run in three, Sonnet never.
- Links, domains, email addresses, @mentions, #tags, file paths, `code` and
  identifiers (`AdCollection`, `snake_case`) are never corrected
  (`protect.rs`), as in Grammarly. The prompt asks the model to leave them,
  and it still offered "frame.io" -> "frame.i", so every fix from either
  pass that would change one is dropped. A fix that only adds around one
  stays ("frame.io" -> "frame.io." is the sentence's period). A domain's
  last label must be lowercase, so "draft.Pick" stays a missing space.
  A lone capital on the end of a lowercase word ("congelationÉ", a
  French-keyboard slip for "?") is not an identifier: protected, the
  model's fix was dropped and the word stayed underlined by nothing.
  A bare domain may change case only, as Grammarly offers "Frame.io" for
  "frame.io"; a link, address or path never, since case can break one.
- A fix that changes nothing the writer can see is dropped (`protect::keep`).
  Editors keep scaffolding in the text: Firefox reports a Coda line as
  "\u{a0}\u{a0}\u{200b}" when empty and "\u{a0}\u{200b}This ...\u{a0}\u{a0}\u{200b}"
  around words. The model took each piece out ("Correct your punctuation"),
  every run, and the cards showed an empty sentence under underlines on
  blank lines and drag handles. Dropped: a change that vanishes once
  invisible characters are left out, every space is a plain space, and the
  spaces opening or closing the line are let go; spaces and invisible
  characters together; anything that takes out an inline object (U+FFFC).
  A doubled space or a tab inside a sentence still shows. The model is
  sent the text without those characters (`protect::visible`) and its
  fixes are mapped back: given "thi is a lest\u{fffc}\u{fffc}" from a
  web editor in Zen, it answered "this is a test" three runs in three,
  each object counted as a changed word, and the answer was refused as a
  rewrite until the writer typed a period. A fix across an object still
  holds it once mapped back, so it is still dropped. The card leaves them
  out of its sentence too: the font drew each as a boxed "OBJ". A line of only
  spaces and invisible characters is blank (`protect::blank`) and is not
  sent to the model. The first fix only covered a zero-width space alone,
  guessed from a screenshot, and missed this; read the field's real text
  (with `\u{..}` escapes) before writing the rule. `squiggle check` asks a
  paragraph at a time, as the app does: whole, Coda's text came back
  without its blank lines and showed nothing.
- The dictionary binds both passes. Harper is given the words; the model
  is not, and with AI on its answer is the whole answer, so a word added
  from a card was corrected again on the next pause. Every fix that would
  change a dictionary word is dropped after the model answers
  (`protect::keep_known`), which also covers cached answers and keeps the
  list on the Mac. A dictionary word in another case may still take the
  dictionary's spelling ("hyprland" -> "Hyprland").
- The menu's spelling reaches the model too: it was Harper-only, and with
  AI on "British" did nothing. American adds nothing to the prompt, so the
  benchmarked prompt is what American users get; the others name the
  spelling with examples. The answer cache is cleared when it changes.
- The dictionary has a window (`dictionary.rs`): a field to add, the list
  to remove from, in the card's look. A Flow-styled console with history
  and stats was mocked first and judged too much; a neutral Mac window
  came next, and then the user asked for nothing macOS-specific, since
  Linux follows. It is borderless, with its own header and close button,
  and says it can become key (`KeyWindow`): a borderless window refuses
  keys otherwise, and the field must take them. An accessory app activates
  itself to give it them. Escape and Command-W close it; no focus ring,
  since the ring is the system accent. `dictionary.txt` stays the store,
  so a word added from a card or typed into the file shows on the next
  open.
- The menu is decided once, in `surface/menu.rs`: `menu(&Status)` returns
  the items, their order, labels and enabled states, and each platform only
  draws them and sends the clicked `Command` back. The two menus had drifted
  (a "Check as I type" the Linux tray ordered differently, a version line of
  its own) because each listed its own items. macOS rebuilds the NSMenu when
  `Status` changes and tags each item with an index into the commands drawn,
  one selector for all.
- The menu has no status line and no on/off switch. The line named the
  model, then said "on this Mac only" or "AI unavailable", then just
  "Checking as you type", which told nothing: running is on, quitting is
  off. Why a model call failed is in the log. It is ordered as most menu
  bar apps are: name and version, what Squiggle is doing, settings, then
  the log and Quit. Pausing is a "Pause ▸" submenu (30 minutes, 1 hour,
  3 hours, until tomorrow), as Slack and Dropbox offer it: a lone "Pause
  for an hour" read oddly. Paused, the menu says "Paused until 15:40" and
  offers Resume in its place. Without Accessibility, "Grant Accessibility
  access…" comes first after the name. A pause lives in memory only, so
  a restart is a fresh start; an `enabled` line in an old config is
  ignored, so it cannot leave Squiggle off with no switch.
- One status does show: "AI suggestions stopped: add OpenRouter credits…"
  (or "check your OpenRouter key…"), under the name, while the model
  refuses for a reason only the writer can end (`router::blocked`: a 402
  or a 401). The key ran out of credits and the log was the only place
  that said so; underlines just stopped. The network or a busy provider
  passes on its own and shows nothing.
- Squiggle is turned off per field, from the card, as in Grammarly: the ⋯
  at the title's end opens "Don't suggest in this field" and the hour's
  pause. A per-app "Turn off in <app>" in the menu came first and was
  taken out; an `off_in` line in an old config is read and ignored. A
  field is known (`off::Place`) by its app, the page's host for a web
  field, its id and its first label; the id or the label is enough,
  since a page may make up ids per load. Those are saved to `off.txt`; a
  field with neither is kept off by its element until Squiggle quits. The
  menu's "Off in N fields" lists them to turn back on, and is hidden
  when there are none. The ⋯'s menu is drawn in the card's look, not a
  system menu, from the same items (`card::MENU`), icons
  (`theme::icon`) and place (`theme::menu_at`) on both platforms, and is
  inside the hover corridor. macOS draws it in panels of its own over the
  card; Linux lays it over the card's words in the card's own surface,
  which needs no second layer surface and already takes the pointer: the
  shortest card is taller than the menu reaches.
- Config and dictionary are read per check, as Flow reads its style per
  dictation, so a setting applies on the next pause rather than the next launch.
- The key never reaches curl's argv; same `-K -` and 0600 body file as Flow.

## Auditing accepts

- `crates/core/tests/apply.rs` accepts every case's fixes in every order
  (one card at a time, as a user would, rebasing the rest after each) and
  checks the text ends exactly as corrected, nothing outside a fix moves,
  and each underline left still sits on its words. Lists, emoji, French
  and neighbouring fixes all pass; the logic was never the problem.
- `squiggle lab` audits the part that was: writing into real fields. A
  second process (`lab-host`) opens a window off screen with an AppKit
  text field, text views (one a TextEdit-style bulleted list) and a web
  page (plain, a list, an inline chip, a textarea). The lab finds each
  field through Accessibility and runs the app's own `Field::bounds` and
  `write::one`/`write::many`; the host answers from AppKit and the DOM
  what the field holds and where each word is drawn, so nothing is judged
  by Accessibility's own account. Keystrokes go to the host's pid only
  (`paste::target`) and the clipboard is put back at the end. Report:
  `target/lab/last.json`. WRONG must stay 0; run it before changing
  `ax.rs`, `write.rs` or `paste.rs`, and add a case for every field that
  misbehaved in use.
- Its first run found a whole-sentence paste across two list items
  re-splitting a web list and leaving a stray space. The app never groups
  across a line (`fix::sentence` stops at one), so it was the lab asking
  more than the app does; the lab now groups as the app does, and the
  paste fallback refuses a span that crosses a line anyway.
- It cannot host Chromium or Gecko, where the bulleted-list fix went
  wrong in use; those still rely on the selected-text check and the log.

## Release checks

- `./sandbox` runs every check and writes `target/sandbox/<time>/report.md`,
  compared with the run before; it exits non-zero when a gate fails. The
  default needs no screen and costs nothing: format, clippy with warnings
  as errors, tests, and the lab's off-screen window. `--ai` adds the
  benchmark on the app's own route (about $0.04), `--models` every
  candidate, `--screen` the browser tabs and the end-to-end run, which
  take the screen for a few minutes.
- Gates: any refused, lost or wrong accept, misplaced underline or case
  that could not run; benchmark precision under 0.93, p95 over 2.5s, F0.5
  more than 0.02 under the last run, or over 2% failed calls (one dropped
  call is the network, and the app falls back for it).
- A case that errors counts against the run. The first sandbox run passed
  while WebKit had dropped its tree mid-run and most web cases never ran;
  the lab now looks for a field for 5s and reports what could not run.
- `squiggle lab --e2e` runs a copy of the app itself (the installed one is
  stopped and restarted) with `SQUIGGLE_STATE` set, so it writes what it
  draws: each fix, its underline, the card's buttons. The lab loads text
  into a browser tab, checks each underline against the page's own word
  rectangle, moves the real pointer to open the card, clicks Accept, checks
  the page, and checks the model's next answer offers nothing on top of
  what was accepted. Screenshots of each step go in the report.

## Offsets

- `Fix` offsets are chars, and so are the `Field` trait's. macOS Accessibility
  counts UTF-16; `utf16_range` is the only conversion. An emoji before a typo is the test case.
- AT-SPI counts chars in GTK4, Chrome and Electron ("Hello 😀 wrold":
  offset 8 is the "w"). Gecko counts UTF-16 and keeps its text in step by
  returning a U+FEFF after every emoji ("Hello 😀\u{feff} wrold", 31
  chars), with a zero-width box.

## Linux surface

- Measured on Hyprland 0.56 against GTK4, Chrome, Electron and Zen, with
  `squiggle doctor --raw` and screenshots of the real screen.
- Apps join the accessibility bus only once `org.a11y.Status.IsEnabled`
  is true. GTK, Qt and Gecko notice at once; Chromium and Electron join
  but build no tree below the window unless started with
  `--force-renderer-accessibility`. `ACCESSIBILITY_ENABLED=1` and
  `ScreenReaderEnabled` were tried and change nothing. The flag is the
  Linux counterpart of `AXManualAccessibility`, except that it needs a
  restart of the app.
- Positions: the Hyprland window's `at` plus the app's character box.
  Wayland hides a window's origin from the app, so Chromium and Gecko
  answer `Screen` from their window's corner and GTK4 refuses `Screen`
  ("Unsupported coordinate space"), answering in `Window` instead. Both
  were pixel-exact on screen. A window capture (`grim -T`) of Chrome is
  offset by its 10px shadow; the screen is not.
- Character boxes, not `GetRangeExtents`: GTK4 answers a range with its
  first character's box, Gecko with partial or empty ones. A range is the
  union of its first and last characters' boxes.
- Focus comes from `object:state-changed:focused` events. A walk of the
  tree per tick is out: a Zen window with 3086 nodes took 0.44s. The walk
  remains for a field already focused before Squiggle was listening.
- An event that names no text field is not trusted to mean "no field":
  Zen reports a childless panel of its window as focused each time the
  window comes forward, and nothing for the field still focused in the
  page. Taken as a control, no field was looked for, and Squiggle showed
  nothing in Zen while saying it ran. The app is asked once, through
  `Collection`, for its focused editable text (5-8ms in Zen, through two
  nested iframes); the walk asks the same first, since a Zen with many
  tabs passed its 5000-node limit.
- A field whose text is only embedded objects is read through them
  (`atspi/blocks.rs`). Chromium gives each block of a rich editor one
  U+FFFC in the field's text and keeps the words in the block: Slack
  4.53's message box read "\u{fffc}", its paragraph "HIlarius! Maiking
  SURE hes legit \u{fffc}", and Squiggle saw a blank field and drew
  nothing. Each object found through `Hypertext` is one line; bounds,
  the selection and the caret go to the line's block, in its own offsets,
  and the focus check stays on the field, since Chromium reports the
  paragraph as not focused. Such a field is always pasted into: its own
  offsets count a block as one character. A field with any text of its
  own between the objects is read as before.
- zbus proxies are built uncached: cached, each one fetches every property
  and subscribes to changes the apps never announce, so values go stale.
- Every call has a 150ms timeout, as on macOS; zbus's default is 25s.
- A Gecko window that has just come forward answers empty text and no
  boxes for a moment while its content's tree arrives; the next poll has
  it.
- Writing: GTK4 and Gecko offer `EditableText`; Chromium and Electron do
  not, so their fixes are pasted over a selection made with
  `Text.SetSelection` and confirmed with `GetSelection` (31-34ms). A paste
  into a window without keyboard focus does nothing: Wayland offers the
  clipboard only to the focused client. Gecko's `EditableText` is not to
  be trusted either: asked to delete and insert in Zen, nothing changed
  within 300ms, and what lands late lands at the caret, not the offset.
- Hyprland 0.56 dispatches in Lua (`hl.dsp.focus({ window = "address:0x…" })`,
  `hl.dsp.send_shortcut({ mods, key, window })`); the `j/` queries over
  the socket are unchanged.
- Accepting follows macOS's rules (`write.rs` is shared). A field is web
  when its app offers no `EditableText` (Chromium, Electron) or a
  `DocumentWeb` is among its first 30 ancestors (Gecko); it is decided once
  per field, and one that cannot be asked about counts as web, since a
  paste needs a confirmed selection and a write does not. Web fields are
  never written directly. Native ones confirm the app's text at the range,
  then `DeleteText` + `InsertText` (length in bytes, as ATK counts it),
  then wait up to `SETTLE` (300ms) for the field to read as expected; a
  field left exactly as it was is pasted from then on.
- A selection is confirmed by `GetSelection(0)` and by the text in it
  (`GetText`), within `SETTLE`. The range reaches the app through
  `app_range`: Gecko's U+FEFF after an emoji is inside a range that ends on
  that emoji, so deleting the emoji takes its padding.
- Two-phase paste, shared with macOS (`write::Paste`: `stage`, then `send`
  or `unstage`). The clipboard is staged before anything is selected, so
  nothing stands between the confirmed selection and Ctrl+V: it was saved
  after, and the gap was the clipboard's round trip. Staging is synchronous
  and bounded (500ms a pipe): save every MIME type, offer the text as plain
  text with `x-kde-passwordManagerHint`, read it back (that read only answers
  once the compositor has it). A save that times out, errors or loses one
  type refuses the fix - a clipboard restored without a type is not the
  user's - and nothing is selected; an empty clipboard is fine, restore
  clears. The restore waits 250ms on a thread and puts the clipboard back
  only if it is still Squiggle's offer (only the types it offered, holding its
  text); one the user copied to meanwhile stays. One paste at a time.
  A fix that only removes text is not pasted: Zen ignores a paste of empty
  text and left the word selected for the next keystroke to replace (Chrome
  deleted it). Backspace goes over the confirmed selection instead, and the
  clipboard is not touched.
- Keystrokes go to the window the field was found in, by Hyprland address,
  never "whatever is in front": pid-addressed, a second window of the same
  app took them. The address is pinned when the focus or window is decided,
  and the front window is read again just before sending; if it differs
  nothing is sent and the paste is refused. Ctrl+V and Ctrl+Z are keycodes
  (`code:55`, `code:52`, XKB = evdev + 8), not keysym names, so a non-Latin
  layout pastes too.
- Web is decided by toolkit first: Gecko and Chromium (`Application.toolkit_name`
  on the bus root) are web throughout. Gecko's `EditableText` lands late and
  at the caret, and Firefox's URL bar has no `DocumentWeb` above it. Otherwise
  no `EditableText` or a `DocumentWeb` above.
- A refusal leaves the field as found: the caret is read before `select_range`
  moves anything and put back, selection dropped, on every exit but a sent
  paste. The selection is asked again right before Ctrl+V, in a field that
  must still have focus: a field keeps its selection when another field in
  the same window is clicked, and the paste would land there.
- A native write that stopped halfway (`DeleteText` took, `InsertText` did
  not, the field reads the text less the range) gets the original text
  inserted back before the outcome is judged; left, it was neither ignored
  nor done and the paste landed on altered text.
- A field is left alone for 1s after any of its calls reaches the 150ms
  deadline (value, frame, boxes, qualifying, web): a hung app froze the main
  loop for another deadline per call. A write's read-back is exempt: during
  the cooldown it read nothing, and a half-finished write went unrepaired.
  A field taken as web only because it did not answer is asked again next
  time, not pasted from for good. The frame that answered a box (Screen
  or Window) is kept per field, so GTK4 is not asked Screen and refused on
  every character. A dropped walk stops at its next node.
- Measured at scale 1 on one monitor only; fractional scale and several
  monitors were not tested.
- A selection starts with the caret, and each step is waited for. Gecko
  keeps the caret as a range of its own beside the one `AddSelection`
  adds and reports only the added one, so every check passed while a
  paste cleared "teh" and typed "the" at the caret: with the caret at 0,
  Zen read "theThe weather was nice and  sun". Moving the caret to the
  range's start first fixes that, but Chromium applies each request later
  in its renderer, and a caret move landing after the selection collapsed
  it ("theteh"). So: caret, wait until it reports there; select, wait for
  the range and its text; then check it still holds 30ms on (`STEADY`).
  Measured after: 15 accepts out of 15 in GTK4, Chrome and Zen starting
  from the caret at 0, each with the clipboard back as it was; "Correct
  the sentence" with four fixes in all three; "café😀" through
  `InsertText` in GTK4.
- Two layer surfaces on the overlay layer, `squiggle-marks` and
  `squiggle-card`. The underline one covers only the field's area (anchored
  top-left, margins and size), shrinking to 1x1 with nothing to draw, so
  fullscreen video and games keep direct scanout; it takes no input (an
  empty input region) and is never unmapped once shown: mapping a layer
  surface again puts it on top, over the card. The card is mapped and
  unmapped, so it lands on top each time, and wants no keyboard, so a
  click on it leaves the field focused.
- Squiggle's stylesheet sits one above the user's (`PRIORITY_USER + 1`). A
  matugen theme in `~/.config/gtk-4.0/gtk.css` painted every window, the
  whole-screen underline layer included, a near-opaque grey, and
  restyled the card's buttons. A rule Squiggle does not reset still
  applies: `window.background { backdrop-filter: blur(20px); }` in the
  user's theme made GTK 4.24 ask Hyprland 0.57 (`ext_background_effect`)
  to blur what lay behind the underline layer, and Slack's text under it
  became an unreadable smear. `window.squiggle` resets it.
- A wrapping label asks for its whole text on one line as its natural
  width, and a layer surface takes its natural size: the sentence card ran
  off its right edge until the label's `max_width_chars` was capped.
- The fallback walk runs on its own thread, at most three times per change
  of window or focus. On the main loop, repeated each second, a Zen window
  with no text field focused (2187 nodes, 0.3s a walk) held the loop up a
  third of the time.
- Layer surfaces would fade in and out; Squiggle sends Hyprland a layer rule
  for the `squiggle` namespaces with `no_anim` when it starts, over the
  socket (`dispatch function() hl.layer_rule({...}) end`).
- The tray is StatusNotifierItem through `ksni` (async-io, blocking API;
  no tokio). It serves D-Bus on its own thread, so a click only sends a
  `Command` down a channel that the 50ms tick drains and applies on the GTK
  thread; the tick pushes `Status` to the tray only when it changed. Spawned
  with `assume_sni_available`: at login Squiggle can start before the shell
  has a tray host, and the icon should appear when the host does. The
  tray only draws `surface::menu` (checkmarks, no radio group, for the
  spelling choices).
- The tray's app name is the desktop entry's name, found among
  `gio::AppInfo::all` by id (`{class}.desktop`, any case): gio 0.22 moved
  `DesktopAppInfo` into a crate this tree does not have. A class with no
  entry reads as its last dot segment capitalised ("com.slack.Slack" ->
  "Slack"). The id stays the class; the name is cached per class.
- The systemd user unit sets no environment. It needs `WAYLAND_DISPLAY` and
  `HYPRLAND_INSTANCE_SIGNATURE` in the user manager, which uwsm provides;
  the installer checks `show-environment` and imports them if missing.
  stderr is appended to `$XDG_STATE_HOME/squiggle/squiggle.log` (`%S`), the
  path "Copy recent log" reads.
- "Personal dictionary…" opens the dictionary window (`dictionary.rs`):
  a field with Add (Enter too) and the list with a remove button on each
  word, in the stylesheet `card::css` appends to, so light or dark and
  matugen-proof like the card. It first opened `dictionary.txt` with
  `xdg-open`, which under the systemd user unit found no handler and did
  nothing. It is a plain toplevel, not a layer surface: only a toplevel
  takes the keyboard on Wayland, and a layer surface that asks for it
  would hold it for every app. Borderless, with its own close button and
  Escape, as on the Mac. One at a time: a second click presents it, with
  the file read again. Hyprland would tile it, so a window rule floats and
  centres it, sent over the socket at startup beside the layer rule and
  matched by title (`Squiggle dictionary`), not class: GTK takes the class
  from the program, shared with the key window. Reading, adding (trim, no
  whitespace inside, no duplicate in any case), removing and the order are
  `squiggle_core::config`, shared with macOS, as are its words
  (`surface::dictionary`). An add or remove asks the engine to check again
  at once; the worker reads the file per check, so nothing caches the old
  list. A word added from a card while the window is open shows only on the
  next present.

## macOS surface

- Polled at 50ms, not driven by `AXObserver`. 100ms made the card feel
  sluggish next to Grammarly's, since hover is a poll too. Notifications are per process
  and per element, several Electron apps never send `AXValueChanged`, and
  observers must be re-registered whenever focus moves.
- Every AX call is IPC into another app; a 150ms messaging timeout keeps a
  hung app from hanging Squiggle.
- The card is one window level above the underlines. Never call
  `setFloatingPanel` on these panels: it resets the level, which put every
  panel on one level and drew underlines over the card.
- Panels are `NonactivatingPanel`, `hidesOnDeactivate = false`. Clicking
  Accept must not steal focus from the field being corrected, and an accessory
  app is never active, so a panel that hides on deactivation never shows.
- Hover is drunk-proof by design. The card, its word and the band between
  them are one corridor (`in_corridor`): inside it nothing closes and no
  word under the card can take the hover. Outside it the card waits 320ms.
  An 8pt gap with a 180ms close made the card vanish on any slow crossing.
- The card follows its fix when the list is rebuilt (`refilter`): same fix,
  untouched; better replacement, redrawn in place on the next tick; gone,
  closed. Closing on every rebuild made a card vanish a second after it
  opened, when the model's answer arrived. Actions apply the fix the card
  shows (`showing`), not the index the button was drawn with.
- Where each fix was drawn is kept until the text changes (`placed`), so a
  word the app fails to place for one tick keeps its underline.
- A card never closes from under the pointer. Two causes looked like
  "hiding for no reason": one unanswered accessibility question (Chromium
  misses the 150ms deadline when busy) was read as focus leaving, and the
  model withdrawing Harper's suggestion closed the card being read. Now a
  field is dropped only after `GRACE` (8) misses in a row, and a withdrawn
  card stays as `ORPHAN` - accepting still works from `card_fix`, but only
  while its range still reads the words the card showed (`card_words`;
  "the card's words moved") - until the pointer leaves. Abnormal closes are logged; `SQUIGGLE_TRACE=1` logs all.
- Only a non-answer waits out `GRACE`. Focus answered as being elsewhere
  (`Focus::Elsewhere`), or another app coming forward, drops the field at
  once: read as misses, a switch to another window left the underlines over
  it for 400ms. `Host::focus` decides it, before any accessibility question:
  macOS compares the process in front with the field's, and a tick also runs
  the moment an app is activated
  (`NSWorkspaceDidActivateApplicationNotification`); Linux compares the
  front window's address with the one the field was found in, which
  Hyprland answers at once. The field already being read is recognised
  (`Focus::Same`) before anything is asked of it, so a tick in a field
  costs one round trip, not five.
- Every app is sent `AXManualAccessibility = true` once when it first comes
  forward (`ax::wake`). Electron and Chromium build no accessibility tree
  without it, which is how Grammarly works in a Claude or Slack text box
  where Squiggle showed nothing. Once is not enough: Slack, woken at 07:48,
  worked until mid-morning and then answered "no focused element" with
  its message box focused, so nothing was checked and nothing was logged;
  asked again, it answered at once. The front app is asked again whenever
  its focus reads as no text field, at most every 5s (`REWAKE`). Gecko browsers (Zen, Firefox) answer that attribute as unsupported and
  are sent `AXEnhancedUserInterface` instead, the one VoiceOver sets: Zen's
  tree fell to its window's three buttons, with the window reported as
  focused, until it was set (6 nodes to 416). Squiggle had worked in Zen only
  while another app kept it set. Other apps are not sent it, since it
  slows their window animations.
- Chromium text areas (the ChatGPT/Codex app, and likely every Electron
  composer) answer `AXBoundsForRange` with an empty rectangle at x = 0 for
  any range. Their `AXStaticText` children answer correctly, so
  `Field::bounds` falls back to those: children are matched to the field's
  value in order by their own text, then asked for the local range. Text
  markers (`AXBoundsForTextMarkerRange`) were tried first and returned the
  whole window. `squiggle doctor --raw` prints the tree when an app misbehaves.
- The underline panel ignores the mouse, so clicks reach the text. Hover is
  detected by polling the pointer position against the word rectangles.
- Writing a fix: select the range, set `AXSelectedText`, then **read the value
  back**. Chromium and Electron accept the write and do nothing. When the read
  disagrees, the fix is pasted over the selection and the clipboard restored
  after 250ms.
- Zen (Gecko) leaves an inline object out of `AXValue` but counts it as one
  U+FFFC in its selection, bounds and `AXStringForRange`. ChatGPT's
  "⌘ AdCollection" chip shifted every range after it one character early:
  accepting "anold" -> "an old" wrote "Pickan old", and each accept broke
  one more character. `Field::value` aligns the value with
  `AXStringForRange` once per text change (`hidden`) and every range sent
  to the app is shifted past those objects.
- A selection is confirmed by its text, not only its range: after the app
  reports the range back, `AXSelectedText` must be the text being fixed.
  In a bulleted list an app counted the bullets in its offsets but not in
  `AXValue`; the range came back exactly as asked, one character early,
  and the paste ate the space before the word (log: "the field holds 200
  chars, expected 201", then the same fix offered again). Before
  selecting, `Field::locate` finds the text in `AXStringForRange` near
  where the offsets say and uses the nearest match, so the fix still
  lands; the log notes the difference and the app. `hidden_objects` now
  takes any character the value lacks, not only U+FFFC, so underlines
  after a bullet are placed right too.
- Zen also applies a selection and an `AXSelectedText` write a moment
  late. Read back at once, the write looked refused and the fix was pasted
  a second time. Both are waited for, up to `SETTLE` (300ms).
- The paste fallback restores every pasteboard item in every type; saving
  only the text wiped a copied screenshot. It is two-phase like Linux's: the
  pasteboard is written before the selection, ⌘V sent right after it is
  confirmed. A fix that only removes text sends Delete over the selection
  instead (`Paste::delete_selection`, shared with Linux): Slack ignored the
  empty paste and the comma stayed (log: "the field holds 68 chars,
  expected 67"). The lab's `removal` case covers it in every field.
- A second accept inside a second of the first is ignored (`expecting`): it
  was made against text the first had already moved.
- A fix never writes over text it has not confirmed. A user saw a whole
  sentence replaced by the correction written twice, not reproducibly:
  an unconfirmed selection (an older, larger one still standing) took the
  write, the read-back disagreed, and the paste wrote it again. Now the
  field is re-read and must still hold the text the fix was found in; the
  selection must be reported back exactly; a write counts as ignored, and
  may be pasted, only when the field is left exactly as it was - any other
  change stops everything; and a field that ignored one write is pasted
  from then on. When in doubt nothing is written, and the log says why
  ("fix not applied: ..."). The first change after an accept is compared
  with what was expected and logged if it differs.
- Web fields (a DOM identity: `AXEditableAncestor` or `AXDOMIdentifier`,
  in browsers and Electron) are never written through `AXSelectedText`;
  the fix is pasted over the confirmed selection, through the page's own
  input handling. Even with the selection confirmed, a direct write in Zen
  replaced a whole sentence with one word ("Correct the sentence", log:
  "a write could not be confirmed"). Native fields keep the direct write.
- An accepted fix is taken off the page at once and is neither offered nor
  written again for `ACCEPTED_FOR` (a minute, in the engine), keyed as a
  dismissal is (the words before it and the change). Slack reported the new text late; in the
  gap the old text still held the fix, it was offered 0.4s later, accepted
  again, and "new" got two commas. It replaced a log line that only noted
  "an accepted fix is offered again".
- The last guard (`audit`): if within a second of an accept the field
  changed and is 10+ characters shorter than both what it held and what
  the fix should leave, ⌘Z takes it back. Typing cannot remove that much
  in that time, and a slow paste leaves the field unchanged, so neither
  triggers it; changing field cancels it.
- "Correct the sentence" applies back to front so each write leaves the
  remaining offsets valid. If the app refuses direct writes, the span the
  fixes cover is pasted once, corrected - a paste per fix would race the
  clipboard. That fallback loses rich formatting inside the span.
- No Accept-all pill in the field's corner: Grammarly has none, and it
  read as clutter. Fixes are accepted from their card.
- A field whose text is its own placeholder, description or title is read as
  empty (`Field::value`). Web composers draw the placeholder as text:
  ChatGPT's empty box in Zen reads "Ask ChatGPT\n" with no
  `AXPlaceholderValue`, the same words in `AXDescription`, and Squiggle
  underlined it. The labels are read once per field, not every poll.
- Terminals are skipped by bundle id, password fields by role. Both are
  deliberate and both are what Grammarly does.
- The Accessibility grant is tied to the signature's designated requirement.
  Ad hoc, that requirement is the build's hash, so every install lost the
  grant. Install now signs with a "Squiggle Local Signing" certificate it makes
  once in the login keychain; the requirement names the certificate and the
  grant survives rebuilds.
- Nothing animates by itself. Core Animation's implicit actions drew every
  new underline sliding in from the panel corner; layer changes go through
  `overlay::still` with actions disabled, panels have no window animation.
  The one motion is deliberate and explicit: the card fades in over 140ms,
  moving its last 3pt towards its word (`fade_in`).
- Underlines are carried across edits (`fix::rebase`) instead of cleared on
  every keystroke, which flickered: a fix the edit touched is dropped, the
  rest shift. Letters typed onto a word's edge count as touching it; a space
  or punctuation does not.
- A dismissal lasts until Squiggle quits, across fields and apps. It was
  cleared whenever focus moved, as Grammarly's per-field dismissals are,
  and a user dismissed a fix, looked at another window, and found it back.
  It is keyed by the three words before the fix on its line plus the
  change, so it holds while the sentence is still being typed and does
  not hide the same change elsewhere.
- One card. Every suggestion - a spelling, a removal, several fixes in a
  sentence, clearer wording - shows its sentence with the change in place
  (struck in red: what goes; bold green on a green fill: what comes), then
  a row of buttons, all 32pt, the main one in ink. Out of its sentence a lone replacement
  did not say why ("it's"), and a removal had to be shown as what was left
  ("me the" for "send me the the link"); `Remove “?”` before that made the
  reader work out the result. The sentence is cut to six words either
  side of the change (`snippet`), and changes one kept word apart are
  shown as one (`bridged`): word by word, a rewrite left "we" standing
  between two struck phrases. A change of spaces alone is shown as ␣.
  Spelling adds "Add to dictionary"; two or more fixes in a sentence get
  "Correct the sentence". Grouping is `fix::sentence`, which splits early
  on abbreviations - a smaller group, never a wrong one. Accepting is a
  button, no longer a click on the replacement, which now sits inside the
  sentence.
- The look is Ink (`surface/theme.rs`, DESIGN.md, `design/`): neutral surfaces,
  ink for the main button, and colour only where it means something: the
  underline, a word going out (red, or blue for clearer wording) and a word
  coming in (green). It replaced Flow's palette, whose green carried every
  job and whose dark card sat on white documents. Struck words were grey
  and new words bold on grey; the change only showed once read. Each word
  has its colour in one place, letters and line in one red: in Dark a pale
  salmon (#FF9592) under a darker red line read as two reds blurring, and
  plain letters under a red line, tried next, lost the change in the
  sentence. Dark's red is coral (#FF7B72, GitHub's deleted code); the
  salmon passed on contrast but read as pink. Corners are 8, 6 and 3; 12, 8
  and 5 read as bubbly. Nothing in it belongs to macOS - no vibrancy, system
  accent or SF font - because the Linux surface draws the same thing.
  Every value either platform draws lives in `surface/theme.rs` and a test
  holds it to `design/tokens.json`: macOS kept its measures, type and shadow
  as constants of its own, and Linux had already drifted (34 high buttons, a
  380 wide card, a 20% tint where macOS has 16%).
  Cards and the dictionary take the system's light or dark appearance when
  they are built; underlines are the same in both, since the app
  underneath can be either.
- The font is Atkinson Hyperlegible Next, bundled in the binary
  (`assets/fonts`, SIL Open Font License) and loaded from its bytes: a
  release is one file, and a bundled font looks the same on every machine.
  Its I, l and 1 differ, which matters when the fix is a capital I. On Linux
  the same bytes are written under `$XDG_RUNTIME_DIR/squiggle/fonts` at start
  and handed to fontconfig as application fonts (`font.rs`) before GTK
  starts, so nothing is installed and uninstall leaves nothing: an install
  script copying them would have needed the files beside the binary in a
  release. The stylesheet names the system sans after it.
- The card's shadow is Squiggle's, in a panel of its own that ignores the
  mouse (`shade`). The system's shadow would differ on Linux, and drawn in
  the card's own panel, its pixels would take clicks meant for the text below
  the card. On Linux it is CSS `box-shadow` on the card, falling inside a
  margin of the card's layer surface (`SHADE`), with the input region cut to
  the card alone, re-applied on every map since a layer surface that is
  mapped again loses it.
- Linux light or dark comes from the portal's `org.freedesktop.appearance
  color-scheme` (`appearance.rs`), read and followed on a thread, applied
  when the next card is built as on macOS. `gtk-application-prefer-dark-theme`
  is only what an app set for itself and says nothing about the desktop.
- On Linux the new words' fill is drawn by a sibling `DrawingArea` beneath
  the label, from the label's own layout (`index_to_pos` per character),
  because Pango's background attribute is a square of the whole line. The
  card fades by GTK opacity and rises through its own margins, never the
  surface's size; Hyprland's layer animation stays off. The hairline is one
  whole pixel, as GTK borders are, where macOS draws half a point.
- The menu bar icon is the mark (`assets/brand/`): a squiggle, the name
  drawn. One stroke 2 units thick, four half circles of radius 2 joined by
  straight runs of 2, in a box of 18 by 8, so stroke and gap are equal and
  the mark is the same turned upside down. Its ends are cut flat, as the
  font cuts its s, l and u: with round ends they were the only soft
  terminals beside the word. The wordmark is lowercase "squiggle" in
  Atkinson Hyperlegible Next SemiBold, the app's own font, as outlines,
  tracked in by 8 per thousand. In the lockup the mark stands on the
  baseline at the x-height with the round letters' overshoot (12 font
  units each way), which makes its stroke 130 font units beside the l's
  122 stem; the gap to the word and the clear space are 4 units. The
  straight runs were chosen among 0, 2, 4 and 6 units: 2 is the length at
  which the stroke matches the font's stem, and longer ones read as a coil
  or as letters. Beside the word it can still read as "nu". In the app
  icon the mark is 558 of the tile's 824 wide. At 22 px a unit is one
  pixel, so the tray's edges land on the pixel grid. It replaced Fold
  (three panels of a folded strip of paper, 0.5.0), which read as
  cardboard and sat oddly in a tray; four rounds of an S in a tile were
  rejected after it as generic. Waves had been ruled out as LanguageTool's
  (a thin line under its letters LT) and Windsurf's; the owner chose the
  squiggle knowing that, since it is the name. Before Fold: a g whose
  tail was the underline shipped in 0.2.0, and `text.badge.checkmark`
  before the rename. The brand is shown on dark, paper (#F1ECE4) on ink
  (#1C1A18); the tile's colour was left as it was and is still open. On
  macOS the icon is a template image (`menubar@2x.png`, 18 x 16 points); a
  Linux tray takes raw ARGB32 pixels from the binary, in the desktop's
  light or dark, following the portal as the card does.
  `assets/brand/build` renders every PNG, the .ico and the tray pixels
  from the SVGs; run it after changing one and commit what it writes. The
  .icns needs macOS's `iconutil`: on Linux the build leaves it as it was
  and says so, and it must be rebuilt on a Mac before a release.
  Tahoe takes the .icns as it is: on macOS 26.5.2 (2026-11-10) the system's
  own icon for a bundle holding only this file (`NSWorkspace.icon(forFile:)`,
  drawn to a PNG) came back fitted to Tahoe's shape with a glass edge on
  the mark, while a bare triangle beside it was put on the grey square. So
  no Icon Composer file is shipped; one would add the clear and tinted
  styles, and needs Xcode's `actool`.
- `squiggle preview [dir]` draws every card and the dictionary to PNG in both
  appearances, from the same code, in an off-screen window. Look at it
  before and after changing how they are drawn.

## Updating

- An installed Squiggle updates from the latest published GitHub Release,
  never by pulling and rebuilding (jb-toolkit R9). The repository is public
  for that reason: a private release needs a token, and a token inside an
  app is a leak.
- The update is the release bundle, and installing it is running the
  bundle's own `packaging/install.sh` (`src/update.rs`). The installer
  stays the only place that knows how Squiggle is installed: a second copy
  of "stop, replace, sign, restart" in Rust would drift from it, and on
  macOS the signing with the local certificate is the part that keeps the
  Accessibility grant.
- The installer stops Squiggle, so it is started outside it: on Linux as a
  transient unit (`systemd-run --user`), since stopping `squiggle.service`
  ends everything in its cgroup; on macOS in a process group of its own.
  Linux was run this way on 2026-11-10 from a real signed bundle: the
  service came back on the new build nine seconds later. The macOS half
  compiles and has not been run: the first release after this one is its
  test.
- The installer copies the new binary beside the old one and renames it
  into place, before nothing else has changed. 0.6.0's installer stopped
  the service and then copied over the binary: `squiggle update` runs from
  that binary, the copy failed with "Text file busy", and Squiggle was left
  stopped (2026-11-10, the first real update, on Linux). The earlier test
  started the installer from a shell, where nothing was running from it.
  From the menu the service is stopped first, so the copy would have
  worked; the command line is the path that broke.
  0.6.1 carries the fix: `squiggle update` took this workstation from
  0.6.0 to 0.6.1 the same day, the service back on the new build in
  seconds. The menu's Update and the macOS path are still to be seen
  working.
- Trust is the signature, nothing else (`squiggle_core::update`). The feed
  is Tauri's `latest.json`; a signature is minisign's, the `.sig` file's
  text as it is. The bundle is verified in memory against the key in
  `packaging/updater.pub`, and those same bytes are unpacked, never a file
  read back. The unpacked binary must then report the version the feed
  named: a signed older bundle offered as a newer one would put a fixed
  bug back. The download address must be one of Squiggle's releases, which
  is tidiness, not security.
- The private key is in the repository's Actions secrets
  (`UPDATER_SIGNING_KEY`, `UPDATER_SIGNING_KEY_PASSWORD`) and the owner's
  password manager, nowhere else. Losing it strands every installed copy:
  they refuse anything it did not sign. The key is given to one step
  only, `scripts/build --sign`, which compiles nothing: during a build
  every dependency's build script and proc-macro runs, and any of them
  could read a key that was in the environment. The build refuses to run
  with the key set, and the signing refuses to run without it.
- Signing is a Rust example (`crates/core/examples/sign.rs`, the `minisign`
  crate as a dev-dependency), so the macOS and Arch runners sign the same
  way with nothing to install and no password prompt to script. The app
  verifies with `minisign-verify`, which has no dependencies. Downloads go
  through curl, as the model calls do.
- Checked at start and daily after: a tray app can run for weeks. A check
  that fails is tried again in 15 minutes, since at login the network is
  often not up yet; before the first release with a feed that is a 404 in
  the log each time. One install runs at a time: a second click emptied
  the folder the first installer was running from. The app waits for the
  installer; it is stopped while it waits, so if the wait ends the
  installer failed without replacing Squiggle, and Update works again. A
  two-minute timer was tried first and could reopen the race under a slow
  installer.
- `scripts/build` makes this platform's bundle in `dist/`, `--sign` its
  signature, and `--feed` writes `latest.json` from the signatures there.
  The release workflow builds on macOS and in Arch, then one job writes the
  feed and drafts the release. Publishing the draft is what ships: installed
  copies read `releases/latest`, which skips drafts and pre-releases.

## Repository

- The git history carries no agent attribution: no `Co-authored-by:` trailers,
  no tool signatures in messages, no agent branches. Agent tooling directories
  stay untracked. Same rule as Flow.
- Squiggle was called Gramly before 1.0. The only places the old name is
  left on purpose: `config::adopt_old_dir`, which moves `~/.config/gramly`
  to `~/.config/squiggle` on first start so settings, the dictionary and
  fields turned off survive the update; the installer, which removes the
  old LaunchAgent and `Gramly.app` (or `gramly.service` and its binary)
  so two apps do not draw two sets of underlines. The repository was
  renamed to `squiggle` and made public on 2026-11-10, when the updater
  needed an address to build in. The new bundle id means macOS asks for
  Accessibility again once.
- main only moves through pull requests. GitHub refuses a direct push to
  it, admins included (branch protection), and a pull request merges only
  once CI's `macos` and `linux` jobs pass on it, up to date with main.
  Merged by rebase: history stays linear and each commit message stays the
  record. Work goes on a branch named for it (`feat/…`, `fix/…`,
  `chore/…`). `.githooks/pre-commit` refuses a commit on main locally
  (`git config core.hooksPath .githooks`, once per clone).
- Versions: root and `crates/core` move together, and the release workflow
  refuses a tag that disagrees with either. Bump with a `chore: bump version
  to X.Y.Z` commit in the pull request, then tag `vX.Y.Z` on main once it
  has merged. Releases are drafts until someone has run the build.

Every agent follows the shared rules in `.toolkit/RULES.md` (managed by jb-toolkit); deliberate exceptions are in `.jb-exceptions`.
