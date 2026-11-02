#!/usr/bin/env bash
# Runs the QML behaviour tests (tests/qml/shell.qml) offscreen, cut off from
# the desktop session (isolated.sh), with the service's config in a scratch
# dir. Fails on a FAIL line, a missing DONE, or any QML warning. Needs
# quickshell and dbus-run-session; no display, no running shell.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

command -v quickshell >/dev/null 2>&1 || { echo "quickshell is required" >&2; exit 1; }

scratch=$(mktemp -d "${TMPDIR:-/tmp}/notchd-qmltest.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
mkdir -- "$scratch/config" "$scratch/data" "$scratch/runtime"
ln -s -- "$root/core" "$scratch/config/core"
ln -s -- "$root/hosts" "$scratch/config/hosts"
ln -s -- "$root/tests/qml/shell.qml" "$scratch/config/shell.qml"
ln -s -- "$root/tests/qml/fixtures" "$scratch/config/fixtures"

# Fake `claude` and `codex` terminal UIs (tests/qml/fixtures/bin), state,
# data, cache and Claude Code's dir in the scratch dir; a fake live bridge and
# a fake components installer.
NOTCHD_QMLTEST_DIR="$scratch/data" XDG_STATE_HOME="$scratch/data/state" XDG_DATA_HOME="$scratch/data/share" XDG_CACHE_HOME="$scratch/data/cache" \
  CLAUDE_CONFIG_DIR="$scratch/data/claude" \
  NOTCHD_LIVE_BRIDGE="$root/tests/qml/fixtures/bin/codex-live" NOTCHD_FAKE_LIVE_ARGS="$scratch/data/live-args" \
  NOTCHD_LOCAL_LIVE_BRIDGE="$root/tests/qml/fixtures/bin/local-live" NOTCHD_FAKE_LOCAL_LIVE_ARGS="$scratch/data/local-live-args" \
  NOTCHD_COMPONENTS_INSTALLER="$root/tests/qml/fixtures/bin/components" NOTCHD_FAKE_COMPONENTS_DIR="$scratch/data/components" \
  PATH="$root/tests/qml/fixtures/bin:$PATH" \
  NOTCHD_FAKE_CLAUDE_ARGS="$scratch/data/claude-args" NOTCHD_FAKE_CODEX_ARGS="$scratch/data/codex-args" "$root/scripts/isolated.sh" "$scratch/runtime" \
  timeout 90 quickshell -p "$scratch/config" --no-color > "$scratch/log" 2>&1 || true

grep -oE '(PASS|FAIL|NOTE) .*' "$scratch/log" || true

# The offscreen backend's lack of window masks is expected (the pointer
# tests use a window).
problems=$(grep -E '^\s*(WARN|ERROR)' "$scratch/log" | grep -vE 'window masks' || true)
if [[ -n $problems ]]; then
  echo "QML warnings during the tests:" >&2
  echo "$problems" >&2
  exit 1
fi
if grep -qE 'FAIL ' "$scratch/log"; then
  exit 1
fi
if ! grep -qE 'DONE$' "$scratch/log"; then
  echo "the QML tests did not finish; log:" >&2
  cat "$scratch/log" >&2
  exit 1
fi
