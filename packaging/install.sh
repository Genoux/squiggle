#!/usr/bin/env bash
# Build Gramly and install it as a menu bar app that starts at login.
#
# Everything lands under $HOME - ~/Applications/Gramly.app and one
# LaunchAgent - with no sudo. Run from a release tarball it uses the binary in
# bin/ and needs no compiler; run from a checkout it builds first. One install
# path for both, as in Flow, so the two cannot drift.
set -euo pipefail

if [ "$(uname -s)" != "Darwin" ]; then
  echo "This installer is for macOS - this is $(uname -s)." >&2
  echo "The Linux build is not ready yet; \`gramly check\` works anywhere cargo does." >&2
  exit 1
fi

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
channel="$(cat "$repo/packaging/channel")"
case "$channel" in
  stable | experimental) ;;
  *) echo "unknown channel $channel - stable or experimental" >&2; exit 1 ;;
esac

apps="$HOME/Applications"
app="$apps/Gramly.app"
agents="$HOME/Library/LaunchAgents"
label="dev.genoux.gramly"
log="$HOME/Library/Logs/gramly.log"

say() { printf '\n\033[1m%s\033[0m\n' "$1"; }

if [ -x "$repo/bin/gramly" ]; then
  binary="$repo/bin/gramly"
  version="$("$binary" --version | awk '{print $2}')"
else
  say "Building"
  cargo build --release --manifest-path "$repo/Cargo.toml"
  # Asked rather than assumed: a target-dir in ~/.cargo/config.toml moves the
  # binaries out of the checkout, which Flow's dev script learned the hard way.
  target="$(cargo metadata --format-version 1 --no-deps --manifest-path "$repo/Cargo.toml" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
  binary="$target/release/gramly"
  version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml" | head -1)"
fi

say "Installing Gramly $version ($channel) to $app"
# Stopped before the bundle is replaced, so launchd does not restart the old
# binary from under the copy.
launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
mkdir -p "$app/Contents/MacOS" "$agents" "$(dirname "$log")"
cp "$binary" "$app/Contents/MacOS/gramly"
sed "s/@VERSION@/$version/g" "$repo/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
# Signed with a local certificate, made once and kept in the login keychain.
# macOS ties the Accessibility grant to the signature's designated
# requirement: ad-hoc signing puts the build's hash in it, so every update
# lost the grant; a certificate puts the certificate there instead, and the
# grant survives every rebuild signed with it.
identity="Gramly Local Signing"
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
    -out "$scratch/id.p12" -passout pass:gramly -name "$identity"
  security import "$scratch/id.p12" -k "$HOME/Library/Keychains/login.keychain-db" \
    -P gramly -T /usr/bin/codesign >/dev/null
  rm -rf "$scratch"
fi
codesign --force --sign "$identity" --identifier "$label" "$app" 2>/dev/null

sed -e "s#@APP@#$app#g" -e "s#@LOG@#$log#g" \
  "$repo/packaging/macos/$label.plist" > "$agents/$label.plist"
launchctl bootstrap "gui/$(id -u)" "$agents/$label.plist"

say "Done"
cat <<MSG
Gramly is in your menu bar and starts at login.

The first time, macOS asks to let Gramly use Accessibility - that is how it
reads the field you are typing in and writes a fix back. Allow it in
System Settings > Privacy & Security > Accessibility.

For AI suggestions, open the menu bar icon > OpenRouter key...
Logs: $log
MSG
