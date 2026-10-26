# Troubleshooting

Start with `gramly doctor` (on a Mac
`~/Applications/Gramly.app/Contents/MacOS/gramly doctor`), click into the field
that is misbehaving, and read what it says.

## Nothing is underlined anywhere

- **Accessibility is not allowed.** The menu says *Needs Accessibility access*.
  System Settings → Privacy & Security → Accessibility → enable Gramly.
- **After an update it stopped working.** macOS ties the grant to the app's
  signature, and each install signs it anew. Remove Gramly from the
  Accessibility list with **−**, then add it back (or toggle it off and on).
- **It is paused.** The menu says *Paused*; menu → *Resume*.

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
*Gramly is on* when it is working, or *AI unavailable* when it is not (the log has the reason).

## Accept does nothing, or pastes something else

Some apps accept a direct write and ignore it; Gramly then pastes the fix over a
selection and restores your clipboard 250ms later. If an app reads the clipboard
slower than that, report it with the app's name.

## Logs

`~/Library/Logs/gramly.log`. The key is never logged.

## Linux

- **Nothing is underlined in Chrome, Slack, Discord or VS Code.** Chromium and
  Electron build no accessibility tree unless started with
  `--force-renderer-accessibility`. `gramly doctor` says so: *focused field:
  none: ... is Chromium with no accessibility tree; start it with
  --force-renderer-accessibility*. Add the flag to
  `~/.config/chrome-flags.conf` or `~/.config/electron-flags.conf` and restart
  the app. Apps that bundle their own Electron (Slack, ChatGPT, Orca) read no
  flag file: put it on the `Exec` line of a copy of their `.desktop` file in
  `~/.local/share/applications`.
- **Nothing happens on GNOME, KDE or Sway.** Gramly is Hyprland only: it asks
  Hyprland's socket where windows and the pointer are, and Wayland offers no
  other way. `gramly doctor` reports *front window: none* outside Hyprland.
- **No tray icon.** The panel needs a StatusNotifierItem host. Gramly keeps
  running without one and the icon appears when the host does.
- **The service fails at login.** `systemctl --user status gramly` and the
  log; usually `WAYLAND_DISPLAY` or `HYPRLAND_INSTANCE_SIGNATURE` is missing
  from systemd's user environment. Have Hyprland run
  `systemctl --user import-environment WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE`
  at startup.
- **Logs** are in `~/.local/state/gramly/gramly.log` (`$XDG_STATE_HOME`), and
  the tray's *Copy recent log* copies the end of it.
