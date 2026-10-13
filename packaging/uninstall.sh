#!/usr/bin/env bash
# Remove Gramly. Settings and your dictionary stay in ~/.config/gramly.
set -euo pipefail
label="dev.genoux.gramly"
launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/$label.plist"
rm -rf "$HOME/Applications/Gramly.app"
echo "Gramly removed. Your settings and dictionary are still in ~/.config/gramly;"
echo "delete that folder too if you want them gone. The Accessibility entry can be"
echo "removed in System Settings > Privacy & Security > Accessibility."
