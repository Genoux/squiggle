#!/usr/bin/env bash
# Remove Gramly. Settings and your dictionary stay in ~/.config/gramly.
set -euo pipefail
if [ "$(uname -s)" = Linux ]; then
  systemctl --user disable --now gramly.service 2>/dev/null || true
  rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/gramly.service" "$HOME/.local/bin/gramly"
  systemctl --user daemon-reload
  echo "Gramly removed. Your settings and dictionary are still in ~/.config/gramly and the"
  echo "log in ~/.local/state/gramly; delete those folders too if you want them gone."
  echo "--force-renderer-accessibility stays in any flag file you added it to: it is"
  echo "harmless without Gramly, and yours to remove."
  exit 0
fi
label="dev.genoux.gramly"
launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/$label.plist"
rm -rf "$HOME/Applications/Gramly.app"
echo "Gramly removed. Your settings and dictionary are still in ~/.config/gramly;"
echo "delete that folder too if you want them gone. The Accessibility entry can be"
echo "removed in System Settings > Privacy & Security > Accessibility."
