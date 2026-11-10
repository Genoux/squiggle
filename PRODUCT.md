# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

People who write a lot in whatever app is open - mail, chat, docs, the
browser - make the same mistakes everyone does, and do not want to pay a
subscription to have them caught.

It is built for its owner, and held to the standard of a stranger installing
it with no one to walk them through it.

## Product Purpose

Mistakes underlined where they were typed. Hover, accept, keep writing.

Success is a fix landing in the field without the caret leaving it, the
writer's voice intact, and a monthly cost the user never thinks about.

## Positioning

Grammarly's interaction - underline, card, accept - without the
account or the subscription. Spelling and grammar are checked on the machine;
the model is the user's own key, asked for small edits, and told to keep
their voice. Clearer wording for a whole sentence is proposed apart from the
corrections, as Grammarly's "Improve your text" is, and never applied unasked.

## Operating Context

A desktop app for macOS and for Linux on Hyprland. It draws its own look on
both, not either system's.

Two surfaces, both drawn over other apps and none of them a window to switch to:

- **Underlines** - red for mistakes, blue for clearer wording, drawn under the words.
- **The card** - on hover: the change, why, Accept / Dismiss / Add to dictionary.

The menu bar icon (the tray on Linux) holds the switches. No Dock icon.

One window exists: the personal dictionary, opened from the menu, to add and
remove words.

Installing is one line in a terminal. Updates arrive in the menu, one click to
install.

## Capabilities and Constraints

**Does:**

- Check the focused field on every pause in typing, locally first, then with the model.
- Apply one fix or all of them without taking focus from the field.
- Learn words; dismiss a suggestion, and it stays dismissed after switching apps.
- Work with no key (local only) and no network.
- Check English and French.
- Update itself from published releases, installing only what its own key signed.

**Must not:**

- Read password fields, or correct terminals.
- Rewrite the user's text unasked. A clearer wording is a proposal on its own
  card, kept apart from corrections, applied only on Accept.
- Steal focus, move the caret, or leave the clipboard changed.
- Need a terminal for daily use.

**Undecided:**

- Who pays for AI suggestions. Today the user brings their own OpenRouter key;
  working with no account at all would mean the owner paying for the model.
- A Mac app to drag into Applications. It needs an Apple Developer ID, not bought.

## Brand Commitments

- The name is Squiggle. It was Gramly before 0.5.0.
- The mark is a squiggle, the name drawn: one stroke, flat ends. The wordmark
  is lowercase "squiggle". Sources and rules are in `assets/brand/`.
- The typeface is Atkinson Hyperlegible Next, in the app and the wordmark.
- The look is called Ink and is the same on every platform. DESIGN.md records it.

## Evidence on Hand

- `design/index.html` and `design/tokens.json`: every token and component.
- `assets/brand/`: the mark, the app icon, the lockup.
- `squiggle preview`: every card and the dictionary, drawn by the app's own code.
- Benchmark numbers for the model and prompt, with dates, in `AGENTS.md`.

There are no users other than the owner, and no testimonials, customers or
press. Nothing of the kind is to be invented.

## Product Principles

- The writer keeps writing: nothing takes focus, moves the caret or needs a window.
- A wrong suggestion costs more trust than a missed one.
- The writer's voice is theirs; a rewrite is offered, never applied.
- Nothing needs explaining: install, first run and updates work for a stranger.
- No account and no subscription.
