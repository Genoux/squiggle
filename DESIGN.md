# Design

Gramly's look is called Ink. It draws on top of other people's apps, so it
stays quiet: neutral surfaces, one ink colour for the main action, and the
underline's colour as the only colour on screen. Nothing in it belongs to
macOS - no vibrancy, system accent or SF font - so the Linux surface draws
the same thing. The values live in `src/macos/theme.rs`; `gramly preview`
draws every card and the dictionary in both appearances.

## Colour

| Token | Light | Dark | Used for |
|---|---|---|---|
| Surface | `#FFFFFF` | `#1C1C1F` | Cards and the dictionary |
| Text | `#18181B` | `#EDEDEF` | The sentence, button labels |
| Secondary | `#66666E` | `#A1A1AA` | Titles, struck words, hints |
| Line | black 13% | white 13% | Outline of a secondary button and the field |
| Fill | `#F0F0F2` | `#2B2B30` | New words, anything under the pointer |
| Ink | `#18181B` | `#EDEDEF` | The main button (`#303036` / `#FFFFFF` under the pointer) |
| Edge | black 12% | white 10% | The hairline around a card |
| Mistake | `#E5484D` | same | Red underline, the card's dot, the mark's tail |
| Clarity | `#4A7BF7` | same | Blue underline and the card's dot |

Cards and the dictionary follow the system's appearance. Underlines and the
grey glow on an accepted word are the same in both: the app underneath can
be light or dark either way.

## Type

Atkinson Hyperlegible Next, bundled (SIL Open Font License). Its I, l and 1
never look alike, which matters when the fix is a capital I.

| Role | Size / weight |
|---|---|
| Card title | 13 / medium, secondary |
| The sentence | 15 / regular on a 24 line; new words semibold |
| Buttons | 13.5 / medium; the main one semibold |
| Dictionary title | 15 / semibold |

## The card

One layout for every suggestion: a dot in the underline's colour and a
title; the sentence with the change in place, struck for what goes and bold
on a fill for what comes; then a row of buttons.

- 340 wide, 16 padding, 12 corners. A hairline edge and a two-layer shadow
  drawn by Gramly, not the system.
- 10 between the title and the sentence, 16 above the buttons, 8 between
  buttons.
- Every button is 32 high, 12 at the sides, 8 corners. The main one is
  solid ink; the others are outlined and fill in under the pointer.
- The sentence is cut to six words either side of the change.
- It fades in over 140ms, moving its last 3pt towards its word. Nothing
  else moves.

## The mark

A g whose tail is the underline. One colour in the menu bar or a tray; with
its tail in the mistake red where Gramly draws in its own colours.
