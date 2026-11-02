#!/usr/bin/env bash
# Renders the scenarios of tests/visual/shell.qml (the settings window, the
# notch on each edge) with Qt's offscreen backend and copies the PNGs to
# build/visual/ (or $1). Fails if a scenario is missing or the QML logged a
# warning, so it doubles as a load check of the core. The view-tree primitives
# (tests/visual/primitives.qml, from the tests/protocol fixtures) render in a
# second run. The media module plays
# tests/visual/demo-player.py's track. Needs quickshell, dbus-run-session,
# ImageMagick, python-dbus and python-gobject; no display, no running shell.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/build/visual"}

for tool in quickshell magick dbus-run-session python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool is required" >&2; exit 1; }
done

scratch=$(mktemp -d "${TMPDIR:-/tmp}/notchd-visual.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
mkdir -- "$scratch/config" "$scratch/shots" "$scratch/demo"
ln -s -- "$root/core" "$scratch/config/core"
ln -s -- "$root/hosts" "$scratch/config/hosts"
ln -s -- "$root/tests/visual/shell.qml" "$scratch/config/shell.qml"

# Cut off from the session, so nothing here touches its sockets, with the
# demo player as the only MPRIS player (with-demo-player.sh).
# Fake `claude` and `codex` terminal UIs, state, data and cache in the
# scratch dir. A fake live bridge (no microphone, no network) and a fake
# components installer (nothing is installed).
NOTCHD_VISUAL_DIR="$scratch/shots" XDG_STATE_HOME="$scratch/shots/state" XDG_DATA_HOME="$scratch/shots/share" XDG_CACHE_HOME="$scratch/shots/cache" \
  CLAUDE_CONFIG_DIR="$scratch/shots/claude" \
  NOTCHD_LIVE_BRIDGE="$root/tests/qml/fixtures/bin/codex-live" \
  NOTCHD_COMPONENTS_INSTALLER="$root/tests/qml/fixtures/bin/components" NOTCHD_FAKE_COMPONENTS_DIR="$scratch/shots/components" \
  PATH="$root/tests/qml/fixtures/bin:$PATH" \
  "$root/scripts/with-demo-player.sh" "$scratch/demo" "$scratch/log" \
  timeout 40 quickshell -p "$scratch/config" --no-color || true

# The primitives of the view tree (core/client), from the protocol fixtures,
# as a config of their own: no D-Bus service, no notch, just the renderer.
mkdir -- "$scratch/prims" "$scratch/prims-shots" "$scratch/prims-runtime"
ln -s -- "$root/core" "$scratch/prims/core"
ln -s -- "$root/tests/visual/primitives.qml" "$scratch/prims/shell.qml"
magick -size 160x160 gradient:'#d08770-#5e81ac' "$scratch/prims-shots/cover.png"
NOTCHD_VISUAL_DIR="$scratch/prims-shots" NOTCHD_FIXTURES="$root/tests/protocol" NOTCHD_COVER="$scratch/prims-shots/cover.png" \
  XDG_STATE_HOME="$scratch/prims-shots/state" XDG_DATA_HOME="$scratch/prims-shots/share" XDG_CACHE_HOME="$scratch/prims-shots/cache" \
  "$root/scripts/isolated.sh" "$scratch/prims-runtime" timeout 40 quickshell -p "$scratch/prims" --no-color >"$scratch/log-prims" 2>&1 || true

# Only Quickshell's own log lines count; the offscreen backend's lack of
# window masks is expected.
problems=$(cat "$scratch/log" "$scratch/log-prims" | grep -E '^\s*(WARN|ERROR)' | grep -vE 'window masks|outside of the config folder' || true)
if [[ -n $problems ]]; then
  echo "QML warnings while rendering:" >&2
  echo "$problems" >&2
  exit 1
fi

scenarios=$(grep -oE 'name: "[a-z-]+"' "$root/tests/visual/shell.qml" | cut -d'"' -f2)
mkdir -p -- "$out"
for name in $scenarios; do
  if [[ ! -s "$scratch/shots/$name.png" ]]; then
    echo "scenario $name produced no image; log:" >&2
    cat "$scratch/log" >&2
    exit 1
  fi
  install -m 0644 -- "$scratch/shots/$name.png" "$out/$name.png"
done
for name in primitives-dark primitives-light; do
  if [[ ! -s "$scratch/prims-shots/$name.png" ]]; then
    echo "scenario $name produced no image; log:" >&2
    cat "$scratch/log-prims" >&2
    exit 1
  fi
  install -m 0644 -- "$scratch/prims-shots/$name.png" "$out/$name.png"
done
echo "visual scenarios in $out"
