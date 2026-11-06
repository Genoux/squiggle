#!/usr/bin/env bash
# Build Squiggle and install it as a menu bar app that starts at login.
#
# Everything lands under $HOME - ~/Applications/Squiggle.app and one
# LaunchAgent on macOS, ~/.local/bin/squiggle and one systemd user unit on
# Linux - with no sudo. Run from a release tarball it uses the binary in
# bin/ and needs no compiler; run from a checkout it builds first. One install
# path for both, as in Flow, so the two cannot drift.
set -euo pipefail

os="$(uname -s)"
case "$os" in
  Darwin) ;;
  Linux)
    # Wayland tells a client nothing about where a window or the pointer is;
    # Hyprland's socket does, and Squiggle asks it. Another compositor would
    # install fine and underline nothing.
    if [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
      echo "Squiggle on Linux needs Hyprland, and this is not a Hyprland session." >&2
      echo "Run the installer from a terminal inside Hyprland." >&2
      exit 1
    fi
    ;;
  *)
    echo "This installer is for macOS and Linux (Hyprland) - this is $os." >&2
    echo "\`squiggle check\` works anywhere cargo does." >&2
    exit 1
    ;;
esac

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
channel="$(cat "$repo/packaging/channel")"
case "$channel" in
  stable | experimental) ;;
  *) echo "unknown channel $channel - stable or experimental" >&2; exit 1 ;;
esac

apps="$HOME/Applications"
app="$apps/Squiggle.app"
agents="$HOME/Library/LaunchAgents"
label="dev.genoux.squiggle"
log="$HOME/Library/Logs/squiggle.log"

say() { printf '\n\033[1m%s\033[0m\n' "$1"; }

flag="--force-renderer-accessibility"

# Whether a flag file already holds the flag, on a line of its own or among
# others, ignoring comments.
has_flag() {
  [ -f "$1" ] && grep -v '^[[:space:]]*#' "$1" | tr -s ' \t' '\n\n' | grep -qxF -- "$flag"
}

# Appended to the file a symlink points at: flag files are often links into a
# dotfiles repo, and replacing the link would orphan the repo's copy.
add_flag() {
  local target
  target="$(readlink -m "$1")"
  mkdir -p "$(dirname "$target")"
  if [ -s "$target" ] && [ -n "$(tail -c1 "$target")" ]; then echo >> "$target"; fi
  echo "$flag" >> "$target"
}

# Chromium and Electron build no accessibility tree unless told to, so
# without the flag they show Squiggle no text field. Asked, never assumed: it
# makes those apps a little slower.
offer_flags() {
  local config="${XDG_CONFIG_HOME:-$HOME/.config}" file answer
  say "Chrome and Electron apps"
  for file in "$config/chrome-flags.conf" "$config/electron-flags.conf"; do
    if has_flag "$file"; then
      echo "$file already has $flag"
    elif [ -t 0 ]; then
      read -r -p "Add $flag to $file? [y/N] " answer
      case "$answer" in
        [yY]*) add_flag "$file"; echo "added to $(readlink -m "$file")" ;;
      esac
    else
      echo "Not asked (no terminal). To add it: echo $flag >> $file"
    fi
  done
  cat <<MSG

Restart those apps afterwards. Slack, ChatGPT, Orca and other apps that ship
their own Electron read neither file: add $flag to the
Exec line of a copy of their .desktop file in ~/.local/share/applications.
MSG
}

# The user manager must hold the session's Wayland and Hyprland variables, or
# the unit starts without them and fails. uwsm puts them there; a plain
# Hyprland start may not.
check_session_environment() {
  local missing="" name
  for name in WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE; do
    systemctl --user show-environment | grep -q "^$name=" || missing="$missing $name"
  done
  [ -z "$missing" ] && return
  systemctl --user import-environment $missing
  cat <<MSG
The systemd user manager did not have:$missing
Imported for this session. For the next login, have Hyprland run
  systemctl --user import-environment WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE
at startup (uwsm does this for you).
MSG
}

install_linux() {
  local bin="$HOME/.local/bin" units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
  local log="${XDG_STATE_HOME:-$HOME/.local/state}/squiggle/squiggle.log"

  say "Installing Squiggle $version ($channel) to $bin/squiggle"
  # Squiggle was Gramly before 1.0: the old unit and binary go, or two would
  # draw two sets of underlines. Settings move on Squiggle's first start.
  if [ -f "$units/gramly.service" ]; then
    systemctl --user disable --now gramly.service 2>/dev/null || true
    rm -f "$units/gramly.service" "$bin/gramly"
  fi
  # Stopped before the binary is replaced: copying over a running one fails.
  systemctl --user stop squiggle.service 2>/dev/null || true
  mkdir -p "$bin" "$units" "$(dirname "$log")"
  cp "$binary" "$bin/squiggle"
  # %h and %S are systemd's home and state directory, the same paths the
  # menu's "Copy recent log" reads.
  cat > "$units/squiggle.service" <<UNIT
[Unit]
Description=Squiggle grammar checker
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=%h/.local/bin/squiggle
Restart=on-failure
RestartSec=3
StandardError=append:%S/squiggle/squiggle.log

[Install]
WantedBy=graphical-session.target
UNIT
  check_session_environment
  systemctl --user daemon-reload
  systemctl --user enable --now squiggle.service
  offer_flags

  say "Done"
  cat <<MSG
Squiggle is in your tray and starts with your session.

Check what it can see in a field with: $bin/squiggle doctor
For AI suggestions, open the tray icon > OpenRouter key...
Logs: $log
MSG
}

if [ -x "$repo/bin/squiggle" ]; then
  binary="$repo/bin/squiggle"
  version="$("$binary" --version | awk '{print $2}')"
else
  say "Building"
  cargo build --release --manifest-path "$repo/Cargo.toml"
  # Asked rather than assumed: a target-dir in ~/.cargo/config.toml moves the
  # binaries out of the checkout, which Flow's dev script learned the hard way.
  target="$(cargo metadata --format-version 1 --no-deps --manifest-path "$repo/Cargo.toml" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
  binary="$target/release/squiggle"
  version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml" | head -1)"
fi

if [ "$os" = Linux ]; then
  install_linux
  exit 0
fi

say "Installing Squiggle $version ($channel) to $app"
# Stopped before the bundle is replaced, so launchd does not restart the old
# binary from under the copy.
launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
# Squiggle was Gramly before 1.0: the old login item and app go, or two would
# draw two sets of underlines. Settings move on Squiggle's first start.
if [ -f "$agents/dev.genoux.gramly.plist" ] || [ -d "$apps/Gramly.app" ]; then
  launchctl bootout "gui/$(id -u)/dev.genoux.gramly" 2>/dev/null || true
  rm -rf "$agents/dev.genoux.gramly.plist" "$apps/Gramly.app"
fi
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$agents" "$(dirname "$log")"
cp "$binary" "$app/Contents/MacOS/squiggle"
cp "$repo/assets/brand/generated/Squiggle.icns" "$app/Contents/Resources/Squiggle.icns"
sed "s/@VERSION@/$version/g" "$repo/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
# Signed with a local certificate, made once and kept in the login keychain.
# macOS ties the Accessibility grant to the signature's designated
# requirement: ad-hoc signing puts the build's hash in it, so every update
# lost the grant; a certificate puts the certificate there instead, and the
# grant survives every rebuild signed with it.
identity="Squiggle Local Signing"
if ! security find-identity -p codesigning 2>/dev/null | grep -q "\"$identity\""; then
  say "Creating the local signing certificate (once)"
  scratch="$(mktemp -d)"
  cat > "$scratch/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $identity
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CNF
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$scratch/cert.cnf" \
    -keyout "$scratch/key.pem" -out "$scratch/cert.pem" 2>/dev/null
  openssl pkcs12 -export -inkey "$scratch/key.pem" -in "$scratch/cert.pem" \
    -out "$scratch/id.p12" -passout pass:squiggle -name "$identity"
  security import "$scratch/id.p12" -k "$HOME/Library/Keychains/login.keychain-db" \
    -P squiggle -T /usr/bin/codesign >/dev/null
  rm -rf "$scratch"
fi
codesign --force --sign "$identity" --identifier "$label" "$app" 2>/dev/null

sed -e "s#@APP@#$app#g" -e "s#@LOG@#$log#g" \
  "$repo/packaging/macos/$label.plist" > "$agents/$label.plist"
launchctl bootstrap "gui/$(id -u)" "$agents/$label.plist"

say "Done"
cat <<MSG
Squiggle is in your menu bar and starts at login.

The first time, macOS asks to let Squiggle use Accessibility - that is how it
reads the field you are typing in and writes a fix back. Allow it in
System Settings > Privacy & Security > Accessibility.

For AI suggestions, open the menu bar icon > OpenRouter key...
Logs: $log
MSG
