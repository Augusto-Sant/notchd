#!/usr/bin/env bash
# Makes sure the notchd daemon runs and prints its socket path: what a host
# (Service.qml, shell.qml) runs at startup. docs/architecture.md, "Distribution".
#
# The binary, first hit wins: $NOTCHD_BIN, `notchd` on PATH,
# $XDG_DATA_HOME/notchd/bin/notchd. None of them: scripts/fetch-daemon.py
# downloads the release into the last place. The instance and config come
# from the environment (NOTCHD_INSTANCE, NOTCHD_CONFIG, NOTCHD_SOCKET), as for
# `notchd` itself; `make dev` sets them so it never meets the user's daemon.
#
# Success: the socket path on stdout, exit 0. Failure: one line of JSON,
# {"error": "..."}, on stdout, exit 1, for the host to show.
set -u

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}/notchd/bin

fail() {
  local msg=${1//\\/\\\\}
  msg=${msg//\"/\\\"}
  msg=${msg//$'\n'/ }
  printf '{"error":"%s"}\n' "$msg"
  exit 1
}

find_bin() {
  if [ -n "${NOTCHD_BIN:-}" ]; then
    [ -x "$NOTCHD_BIN" ] && { printf '%s\n' "$NOTCHD_BIN"; return 0; }
    return 1
  fi
  command -v notchd 2>/dev/null && return 0
  [ -x "$data/notchd" ] && { printf '%s\n' "$data/notchd"; return 0; }
  return 1
}

bin=$(find_bin) || {
  if [ -n "${NOTCHD_BIN:-}" ]; then
    fail "NOTCHD_BIN is not executable: $NOTCHD_BIN"
  fi
  [ -f "$here/fetch-daemon.py" ] || fail "notchd not found"
  fetched=$(python3 "$here/fetch-daemon.py" --dest "$data") || {
    # fetch-daemon.py prints its own JSON error: pass it on as it is.
    printf '%s\n' "$fetched"
    exit 1
  }
  bin=$(printf '%s\n' "$fetched" | tail -n 1)
  [ -x "$bin" ] || fail "fetched notchd is not executable: $bin"
}

out=$("$bin" daemon --ensure 2>&1) || fail "${out##*$'\n'}"
printf '%s\n' "${out##*$'\n'}"
