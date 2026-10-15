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
  (`paste.rs`), the Ink palette and font (`theme.rs`), and
  `gramly preview` (`preview.rs`).
- `design/` is the design system: `index.html` shows every token and
  component, `tokens.json` holds the values for the code and for the Linux
  surface. It lives in the repo because the repo is private and the page must
  change with the code; tests fail when either disagrees with the code.
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
  three runs each over a dozen real messages with `gramly check`, which
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
  has not answered within `MODEL_LATE` (4s).
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
  (`cargo run --release -p gramly-core --example bench`): every route in
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
- "Turn off in <app>" stores bundle ids in `off_in`, checked against the
  app in front each tick; "Pause for an hour" lives in memory only, so a
  restart is a fresh start. Both show in the menu's status line.
- Config and dictionary are read per check, as Flow reads its style per
  dictation, so a setting applies on the next pause rather than the next launch.
- The key never reaches curl's argv; same `-K -` and 0600 body file as Flow.

## Auditing accepts

- `crates/core/tests/apply.rs` accepts every case's fixes in every order
  (one card at a time, as a user would, rebasing the rest after each) and
  checks the text ends exactly as corrected, nothing outside a fix moves,
  and each underline left still sits on its words. Lists, emoji, French
  and neighbouring fixes all pass; the logic was never the problem.
- `gramly lab` audits the part that was: writing into real fields. A
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
- `gramly lab --e2e` runs a copy of the app itself (the installed one is
  stopped and restarted) with `GRAMLY_STATE` set, so it writes what it
  draws: each fix, its underline, the card's buttons. The lab loads text
  into a browser tab, checks each underline against the page's own word
  rectangle, moves the real pointer to open the card, clicks Accept, checks
  the page, and checks the model's next answer offers nothing on top of
  what was accepted. Screenshots of each step go in the report.

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
  card stays as `ORPHAN` - accepting still works from `card_fix` - until the
  pointer leaves. Abnormal closes are logged; `GRAMLY_TRACE=1` logs all.
- Every app is sent `AXManualAccessibility = true` once when it first comes
  forward (`ax::wake`). Electron and Chromium build no accessibility tree
  without it, which is how Grammarly works in a Claude or Slack text box
  where Gramly showed nothing.
- Chromium text areas (the ChatGPT/Codex app, and likely every Electron
  composer) answer `AXBoundsForRange` with an empty rectangle at x = 0 for
  any range. Their `AXStaticText` children answer correctly, so
  `Field::bounds` falls back to those: children are matched to the field's
  value in order by their own text, then asked for the local range. Text
  markers (`AXBoundsForTextMarkerRange`) were tried first and returned the
  whole window. `gramly doctor --raw` prints the tree when an app misbehaves.
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
  only the text wiped a copied screenshot.
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
  `AXPlaceholderValue`, the same words in `AXDescription`, and Gramly
  underlined it. The labels are read once per field, not every poll.
- Terminals are skipped by bundle id, password fields by role. Both are
  deliberate and both are what Grammarly does.
- The Accessibility grant is tied to the signature's designated requirement.
  Ad hoc, that requirement is the build's hash, so every install lost the
  grant. Install now signs with a "Gramly Local Signing" certificate it makes
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
- A dismissal lasts until Gramly quits, across fields and apps. It was
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
- The look is Ink (`theme.rs`, DESIGN.md, `design/`): neutral surfaces,
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
  accent or SF font - because the Linux surface will draw the same thing.
  Cards and the dictionary take the system's light or dark appearance when
  they are built; underlines are the same in both, since the app
  underneath can be either.
- The font is Atkinson Hyperlegible Next, bundled in the binary
  (`assets/fonts`, SIL Open Font License) and loaded from its bytes: a
  release is one file, and a bundled font looks the same on every machine.
  Its I, l and 1 differ, which matters when the fix is a capital I.
- The card's shadow is Gramly's, in a panel of its own that ignores the
  mouse (`shade`). The system's shadow would differ on Linux, and drawn in
  the card's own panel, its pixels would take clicks meant for the text below
  the card.
- The menu bar icon is the system's spell-check symbol
  (`text.badge.checkmark`), not a logo. A drawn mark (a g whose tail is the
  underline) shipped in 0.2.0 and was taken out: the name will change, and
  an alpha needs no brand. A Linux tray takes the icon theme's
  `tools-check-spelling`.
- `gramly preview [dir]` draws every card and the dictionary to PNG in both
  appearances, from the same code, in an off-screen window. Look at it
  before and after changing how they are drawn.

## Repository

- The git history carries no agent attribution: no `Co-authored-by:` trailers,
  no tool signatures in messages, no agent branches. Agent tooling directories
  stay untracked. Same rule as Flow.
- Versions: root and `crates/core` move together, and the release workflow
  refuses a tag that disagrees with either. Bump with a `chore: bump version
  to X.Y.Z` commit, then tag `vX.Y.Z`. Releases are drafts until someone has
  run the build.
