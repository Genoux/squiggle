# Product

## Platform

macos, then linux (Wayland, Hyprland)

## Users

People who write a lot in whatever app is open - mail, chat, docs, the
browser - make the same mistakes everyone does, and do not want to pay a
subscription to have them caught.

## Product Purpose

Mistakes underlined where they were typed. Hover, accept, keep writing.

Success is a fix landing in the field without the caret leaving it, the
writer's voice intact, and a monthly cost the user never thinks about.

## Positioning

Grammarly's interaction - underline, card, accept, accept all - without the
account, the subscription, or the rewriting. Spelling and grammar are checked
on the machine; the model is the user's own key, asked for small edits, and
told to keep their voice.

## Operating Context

Three surfaces, all drawn over other apps and none of them a window to switch to:

- **Underlines** - red for mistakes, blue for clarity, drawn under the words.
- **The card** - on hover: the change, why, Accept / Dismiss / Add to dictionary.
- **The pill** - in the field's corner: the count and Accept all.

The menu bar icon holds the switches. No Dock icon.

## Capabilities and Constraints

**Does:**

- Check the focused field on every pause in typing, locally first, then with the model.
- Apply one fix or all of them without taking focus from the field.
- Learn words; dismiss a suggestion for the rest of that field.
- Work with no key (local only) and no network.

**Must not:**

- Read password fields, or correct terminals.
- Rewrite the user's text for style. Edits, not paraphrases.
- Steal focus, move the caret, or leave the clipboard changed.
- Need a terminal for daily use.
