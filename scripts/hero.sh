#!/usr/bin/env bash
# Renders the README image (docs/images/hero.webp, or $1) from
# tests/visual/hero.qml: the real notch in four states, offscreen, on a
# private D-Bus where the only media player is tests/visual/demo-player.py,
# with the demo agents of tests/visual/bin.
#
# The tiles show WALLPAPER, by default Omarchy's Gruvbox "The Backwater"
# (the wallpaper is not in the repo; only the rendered image is). Without
# it they fall back to a gradient.
# Needs quickshell, ImageMagick, python-dbus, python-gobject and
# JetBrainsMono Nerd Font.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/docs/images/hero.webp"}
wallpaper=${WALLPAPER:-"${OMARCHY_PATH:-/usr/share/omarchy}/themes/gruvbox/backgrounds/1-the-backwater.jpg"}
if [[ ! -r $wallpaper ]]; then
  echo "wallpaper not found ($wallpaper); using a gradient. Set WALLPAPER=<image>." >&2
  wallpaper=""
fi
export NOTCHD_HERO_WALLPAPER=$wallpaper

for tool in quickshell magick dbus-run-session python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool is required" >&2; exit 1; }
done

scratch=$(mktemp -d "${TMPDIR:-/tmp}/notchd-hero.XXXXXX")
trap 'pkill -f -- "--socket $scratch/" 2>/dev/null || true; rm -rf -- "$scratch"' EXIT
mkdir -- "$scratch/config" "$scratch/out" "$scratch/demo" "$scratch/sock"
ln -s -- "$root/core" "$scratch/config/core"
ln -s -- "$root/hosts" "$scratch/config/hosts"
ln -s -- "$root/tests/visual/hero.qml" "$scratch/config/shell.qml"
cp -- "$root/tests/qml/TestService.qml" "$scratch/config/TestService.qml"

# Cut off from the session, with the demo player as the only MPRIS player,
# and the demo `claude` and `codex` (tests/visual/bin), state in the scratch dir.
NOTCHD_TEST_DAEMON="$root/native/target/release/notchd" NOTCHD_TEST_SOCKETS="$scratch/sock" NOTCHD_TEST_HOOKS=1 \
  NOTCHD_VISUAL_DIR="$scratch/out" NOTCHD_HERO_FILE="$scratch/out/hero.png" \
  XDG_STATE_HOME="$scratch/out/state" CLAUDE_CONFIG_DIR="$scratch/out/claude" \
  PATH="$root/tests/visual/bin:$PATH" \
  "$root/scripts/with-demo-player.sh" "$scratch/demo" "$scratch/log" \
  timeout 30 quickshell -p "$scratch/config" --no-color || true

# Only Quickshell's own log lines; GLib/AT-SPI chatter about the private bus
# starts with "** (" and is not ours.
problems=$(grep -E '^\s*(WARN|ERROR)' "$scratch/log" | grep -vE 'window masks|outside of the config folder' || true)
if [[ -n $problems ]]; then
  echo "QML warnings while rendering:" >&2
  echo "$problems" >&2
  exit 1
fi
[[ -s "$scratch/out/hero.png" ]] || { echo "no image; log:" >&2; cat "$scratch/log" >&2; exit 1; }

mkdir -p -- "$(dirname -- "$out")"
# WebP: the painted wallpaper makes a PNG weigh ~1.7 MB; at quality 90 this
# is ~400 KB with text as crisp. The image is committed.
magick "$scratch/out/hero.png" -strip -quality 90 "$out"
echo "hero image: $out"
