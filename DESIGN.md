# Design

Squiggle's look is called Ink. It draws on top of other people's apps, so it
stays quiet: neutral surfaces, one ink colour for the main action, and colour
only where it means something - the underline, the words a fix takes out
(red) and puts in (green). Nothing in it belongs to macOS - no vibrancy,
system accent or SF font - so the Linux surface draws the same thing.

The design system is `design/index.html`: every token and component, in both
appearances. Its values are `design/tokens.json`, which `cargo test` checks
against `src/macos/theme.rs`, `overlay.rs` and `dictionary.rs`, and against the
page itself. `squiggle preview` draws every card and the dictionary from the
app's own code.

## Colour

| Token | Light | Dark | Used for |
|---|---|---|---|
| Surface | `#FFFFFF` | `#1C1C1F` | Cards and the dictionary |
| Text | `#18181B` | `#EDEDEF` | The sentence, button labels |
| Secondary | `#66666E` | `#A1A1AA` | Titles, hints |
| Line | black 13% | white 13% | Outline of a secondary button and the field |
| Fill | `#F0F0F2` | `#2B2B30` | A secondary button under the pointer |
| Ink | `#18181B` | `#EDEDEF` | The main button (`#303036` / `#FFFFFF` under the pointer) |
| Edge | black 12% | white 10% | The hairline around a card |
| Gone | `#CE2C31` | `#FF7B72` | A word a correction takes out, and the line through it |
| Gone, wording | `#2F5BD3` | `#9DB5FF` | A word clearer wording takes out |
| New | `#18794E` | `#3DD68C` | A word a fix puts in, bold |
| New fill | `#30A46C` 15% | `#30A46C` 22% | Behind it |
| Mistake | `#E5484D` | same | Red underline, the card's dot |
| Clarity | `#4A7BF7` | same | Blue underline and the card's dot |
| Accepted | `#30A46C` 34% | same | The glow on a word just accepted |

Cards and the dictionary follow the system's appearance. Underlines and the
glow are the same in both: the app underneath can be light or dark either
way. Each changed word has its colour in one place: an old word's letters and
line are the same red, a new word is green on green.

## Type

Atkinson Hyperlegible Next, bundled (SIL Open Font License). Its I, l and 1
never look alike, which matters when the fix is a capital I.

| Role | Size / weight |
|---|---|
| Card title | 13 / medium, secondary |
| The sentence | 15 / regular on a 24 line; new words semibold |
| Buttons | 13.5 / medium; the main one semibold |
| Dictionary title | 15 / semibold |

## Corners

8 for the card and the dictionary, 6 for buttons and the field, 3 for a new
word's fill and the glow on a word.

## The card

One layout for every suggestion: a dot in the underline's colour and a
title; the sentence with the change in place, struck in red for what goes
and bold green on a fill for what comes; then a row of buttons.

- 340 wide, 16 padding. A hairline edge and a two-layer shadow drawn by
  Squiggle, not the system.
- 10 between the title and the sentence, 16 above the buttons, 8 between
  buttons.
- Every button is 32 high, 12 at the sides. The main one is solid ink; the
  others are outlined and fill in under the pointer.
- The sentence is cut to six words either side of the change.
- A ⋯ at the title's end, 24 square, opens the card's menu: 240 wide, 4
  padding, items 32 high with an icon, in the card's surface and shadow.
- It fades in over 140ms, moving its last 3pt towards its word. Nothing
  else moves.

## The menu bar

The system's spell-check symbol, not a logo: the name will change before
Squiggle has a mark. A Linux tray takes the icon theme's `tools-check-spelling`.
