# Troubleshooting

Start with `gramly doctor` (or
`~/Applications/Gramly.app/Contents/MacOS/gramly doctor`), click into the field
that is misbehaving, and read what it says.

## Nothing is underlined anywhere

- **Accessibility is not allowed.** The menu says *Needs Accessibility access*.
  System Settings → Privacy & Security → Accessibility → enable Gramly.
- **After an update it stopped working.** macOS ties the grant to the app's
  signature, and each install signs it anew. Remove Gramly from the
  Accessibility list with **−**, then add it back (or toggle it off and on).
- **It is paused.** Menu bar icon → *Check as I type*.

## Nothing is underlined in one app

`gramly doctor` in that app:

- *focused field: none* - the app does not expose its text to Accessibility,
  or it is a terminal (skipped on purpose).
- *word positions: NOT reported* - the app gives the text but not where it is
  drawn, so there is nowhere to put an underline.
- **Chrome / Electron (Slack, Discord, VS Code):** usually work, but some only
  expose text while an assistive app is running. If doctor finds no field,
  quit and reopen the app once Gramly is running.

## Only spelling is caught, never the subtler mistakes

AI suggestions need a key: menu → *OpenRouter key…*. The first menu line says
*Checking with Claude Haiku 5.5* when it is working, or the reason when it is not.

## Accept does nothing, or pastes something else

Some apps accept a direct write and ignore it; Gramly then pastes the fix over a
selection and restores your clipboard 250ms later. If an app reads the clipboard
slower than that, report it with the app's name.

## Logs

`~/Library/Logs/gramly.log`. The key is never logged.
