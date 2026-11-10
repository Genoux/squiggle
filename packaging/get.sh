#!/usr/bin/env bash
# Installs the latest Squiggle release: one line, no checkout, no build.
#
#   curl -fsSL https://github.com/Genoux/squiggle/releases/latest/download/install.sh | bash
#
# Published with every release under that name. The download is trusted as
# far as HTTPS to GitHub is, like any download from a release page; from
# then on the app only installs updates its own key signed.
set -euo pipefail

# In a function, called on the last line: piped into bash, a download cut
# short must not run half a script.
main() {
  local feed="https://github.com/Genoux/squiggle/releases/latest/download/latest.json"
  local platform url scratch
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) platform=darwin-aarch64 ;;
    Linux-x86_64) platform=linux-x86_64 ;;
    *)
      echo "Squiggle has no release for $(uname -s) on $(uname -m) yet." >&2
      exit 1
      ;;
  esac
  url="$(curl -fsSL --proto '=https' "$feed" | tr -d '\n' \
    | grep -o "\"url\": *\"[^\"]*$platform[^\"]*\"" | sed 's/.*"\(https[^"]*\)"/\1/')"
  if [ -z "$url" ]; then
    echo "The latest Squiggle release has no bundle for $platform." >&2
    exit 1
  fi
  scratch="$(mktemp -d)"
  # The path is written into the trap now: it runs after this function has
  # returned, when a local no longer exists.
  trap "rm -rf '$scratch'" EXIT
  curl -fsSL --proto '=https' "$url" | tar -xz -C "$scratch"
  bash "$scratch"/squiggle-*/packaging/install.sh
}

main
