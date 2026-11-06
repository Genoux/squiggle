#!/usr/bin/env bash
# Remove Squiggle. Settings and your dictionary stay in ~/.config/squiggle.
set -euo pipefail
if [ "$(uname -s)" = Linux ]; then
  systemctl --user disable --now squiggle.service 2>/dev/null || true
  rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/squiggle.service" "$HOME/.local/bin/squiggle"
  systemctl --user daemon-reload
  echo "Squiggle removed. Your settings and dictionary are still in ~/.config/squiggle and the"
  echo "log in ~/.local/state/squiggle; delete those folders too if you want them gone."
  echo "--force-renderer-accessibility stays in any flag file you added it to: it is"
  echo "harmless without Squiggle, and yours to remove."
  exit 0
fi
label="dev.genoux.squiggle"
launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/$label.plist"
rm -rf "$HOME/Applications/Squiggle.app"
echo "Squiggle removed. Your settings and dictionary are still in ~/.config/squiggle;"
echo "delete that folder too if you want them gone. The Accessibility entry can be"
echo "removed in System Settings > Privacy & Security > Accessibility."
