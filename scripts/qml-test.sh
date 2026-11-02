#!/usr/bin/env bash
# Runs the QML behaviour tests (tests/qml/shell.qml) offscreen, cut off from
# the desktop session (isolated.sh), with the service's config in a scratch
# dir. Fails on a FAIL line, a missing DONE, or any QML warning. Needs
# quickshell and dbus-run-session; no display, no running shell.
#
# The cases are mostly waits on timers, so they run in QMLTEST_JOBS shards at
# once (default: the CPUs, at most 8), each its own Quickshell and scratch
# dir, longest first by the last run's times (build/times/qml.tsv, which this
# writes). Two more shards: the daemon client's (client.qml, a fake daemon) and
# the shadow test's (shadow.qml, the real daemon, which `make qmltest` builds).
# QMLTEST_ONLY=<text> runs only the cases whose name contains it;
# QMLTEST_VERBOSE=1 lists the cases that passed too; QMLTEST_KEEP=1 keeps the scratch dir.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
times="$root/build/times/qml.tsv"

command -v quickshell >/dev/null 2>&1 || { echo "quickshell is required" >&2; exit 1; }

jobs=${QMLTEST_JOBS:-$(( $(nproc) < 8 ? $(nproc) : 8 ))}
[[ -n ${QMLTEST_ONLY:-} ]] && jobs=1

scratch=$(mktemp -d "${TMPDIR:-/tmp}/notchd-qmltest.XXXXXX")
trap 'pkill -f -- "--socket $scratch/" 2>/dev/null || true; [[ -n ${QMLTEST_KEEP:-} ]] || rm -rf -- "$scratch"' EXIT

# One line per shard: the indexes of its cases. Longest first, each to the
# shard with the least to do; a case not timed yet counts as 2 s.
python3 - "$root/tests/qml/shell.qml" "$times" "$jobs" > "$scratch/shards" <<'EOF'
import re, sys
qml, times, jobs = sys.argv[1], sys.argv[2], int(sys.argv[3])
names = [m.replace('\\"', '"') for m in re.findall(r'^\s*\{ name: "(.*)", run: root\.\w+ \},?$', open(qml).read(), re.M)]
last = {}
try:
    for line in open(times):
        secs, name = line.rstrip("\n").split("\t", 1)
        last[name] = float(secs)
except (OSError, ValueError):
    pass
shards = [[0.0, []] for _ in range(jobs)]
for i in sorted(range(len(names)), key=lambda i: -last.get(names[i], 2.0)):
    s = min(shards, key=lambda s: s[0])
    s[0] += last.get(names[i], 2.0)
    s[1].append(i)
for _, idx in shards:
    if idx:
        print(",".join(map(str, sorted(idx))))
EOF

run_shard() {
  local dir=$1 cases=$2
  mkdir -- "$dir" "$dir/config" "$dir/data" "$dir/runtime" "$dir/sock"
  ln -s -- "$root/core" "$dir/config/core"
  ln -s -- "$root/hosts" "$dir/config/hosts"
  ln -s -- "$root/tests/qml/shell.qml" "$dir/config/shell.qml"
  ln -s -- "$root/tests/qml/fixtures" "$dir/config/fixtures"
  cp -- "$root/tests/qml/TestService.qml" "$dir/config/TestService.qml"
  # Fake `claude` and `codex` terminal UIs (tests/qml/fixtures/bin), state,
  # data, cache and Claude Code's dir in the scratch dir; a fake live bridge
  # and a fake components installer.
  NOTCHD_TEST_DAEMON="$daemon_bin" NOTCHD_TEST_SOCKETS="$dir/sock" \
    NOTCHD_QMLTEST_CASES="$cases" NOTCHD_QMLTEST_ONLY="${QMLTEST_ONLY:-}" \
    NOTCHD_QMLTEST_DIR="$dir/data" XDG_STATE_HOME="$dir/data/state" XDG_DATA_HOME="$dir/data/share" XDG_CACHE_HOME="$dir/data/cache" \
    CLAUDE_CONFIG_DIR="$dir/data/claude" \
    NOTCHD_LIVE_BRIDGE="$root/tests/qml/fixtures/bin/codex-live" NOTCHD_FAKE_LIVE_ARGS="$dir/data/live-args" \
    NOTCHD_LOCAL_LIVE_BRIDGE="$root/tests/qml/fixtures/bin/local-live" NOTCHD_FAKE_LOCAL_LIVE_ARGS="$dir/data/local-live-args" \
    NOTCHD_COMPONENTS_INSTALLER="$root/tests/qml/fixtures/bin/components" NOTCHD_FAKE_COMPONENTS_DIR="$dir/data/components" \
    PATH="$root/tests/qml/fixtures/bin:$PATH" \
    NOTCHD_FAKE_CLAUDE_ARGS="$dir/data/claude-args" NOTCHD_FAKE_CODEX_ARGS="$dir/data/codex-args" "$root/scripts/isolated.sh" "$dir/runtime" \
    timeout -k 5 90 quickshell -p "$dir/config" --no-color > "$dir/log" 2>&1 || true
  # The services' daemons (tests/qml/TestService.qml) outlive the shell: stop them.
  pkill -f -- "--socket $dir/sock" 2>/dev/null || true
}

# The client's cases (tests/qml/client.qml) are one more shard, with the fake
# daemon (tests/qml/fake-daemon.mjs) on a socket in the shard's own scratch dir.
# Cut off like the others (isolated.sh); the daemon is killed when the shard
# ends, whatever happens, and every wait is bounded.
run_client_shard() {
  local dir=$1 daemon_pid="" i
  mkdir -- "$dir" "$dir/config" "$dir/data" "$dir/runtime" "$dir/sock"
  ln -s -- "$root/core" "$dir/config/core"
  ln -s -- "$root/tests/qml/client.qml" "$dir/config/shell.qml"
  node "$root/tests/qml/fake-daemon.mjs" "$dir/sock/d.sock" > "$dir/daemon.log" 2>&1 &
  daemon_pid=$!
  for ((i = 0; i < 100; i++)); do
    [[ -S $dir/sock/d.sock.ctl ]] && break
    sleep 0.05
  done
  NOTCHD_QMLTEST_SOCKET="$dir/sock/d.sock" NOTCHD_QMLTEST_ONLY="${QMLTEST_ONLY:-}" \
    NOTCHD_QMLTEST_DIR="$dir/data" XDG_STATE_HOME="$dir/data/state" XDG_DATA_HOME="$dir/data/share" XDG_CACHE_HOME="$dir/data/cache" \
    "$root/scripts/isolated.sh" "$dir/runtime" timeout -k 5 90 quickshell -p "$dir/config" --no-color > "$dir/log" 2>&1 || true
  kill "$daemon_pid" 2>/dev/null || true
  wait "$daemon_pid" 2>/dev/null || true
}

# The shadow shard (tests/qml/shadow.qml): the real daemon beside the QML
# service, given the same inputs and compared. The test starts its own daemon
# per scenario (NOTCHD_TEST_HOOKS=1, a socket in this shard's scratch dir) and
# stops it; whatever is left is killed here when the shard ends. The binary is
# `make daemon`'s (NOTCHD_BIN overrides).
daemon_bin=${NOTCHD_BIN:-$root/native/target/release/notchd}
run_shadow_shard() {
  local dir=$1
  mkdir -- "$dir" "$dir/config" "$dir/data" "$dir/runtime" "$dir/sock"
  ln -s -- "$root/core" "$dir/config/core"
  ln -s -- "$root/tests/qml/shadow.qml" "$dir/config/shell.qml"
  ln -s -- "$root/tests/qml/fixtures" "$dir/config/fixtures"
  NOTCHD_QMLTEST_SOCKETS="$dir/sock" NOTCHD_QMLTEST_DAEMON="$daemon_bin" NOTCHD_QMLTEST_ONLY="${QMLTEST_ONLY:-}" \
    NOTCHD_QMLTEST_DIR="$dir/data" XDG_STATE_HOME="$dir/data/state" XDG_DATA_HOME="$dir/data/share" XDG_CACHE_HOME="$dir/data/cache" \
    "$root/scripts/isolated.sh" "$dir/runtime" timeout -k 5 120 quickshell -p "$dir/config" --no-color > "$dir/log" 2>&1 || true
  pkill -f -- "--socket $dir/sock" 2>/dev/null || true
}

command -v node >/dev/null 2>&1 || { echo "node is required" >&2; exit 1; }
[[ -x $daemon_bin ]] || { echo "no daemon at $daemon_bin: run make daemon" >&2; exit 1; }
n=0
pids=()
while IFS= read -r cases; do
  run_shard "$scratch/$n" "$cases" &
  pids+=("$!")
  n=$((n + 1))
done < "$scratch/shards"
run_client_shard "$scratch/$n" &
pids+=("$!")
n=$((n + 1))
run_shadow_shard "$scratch/$n" &
pids+=("$!")
n=$((n + 1))
wait "${pids[@]}"

status=0
for ((i = 0; i < n; i++)); do
  log="$scratch/$i/log"
  if [[ -n ${QMLTEST_VERBOSE:-} ]]; then
    grep -oE '(PASS|FAIL|NOTE) .*' "$log" || true
  else
    grep -oE '(FAIL|NOTE) .*' "$log" || true
  fi
  # The offscreen backend's lack of window masks is expected (the pointer
  # tests use a window).
  problems=$(grep -E '^\s*(WARN|ERROR)' "$log" | grep -vE 'window masks' || true)
  # The client's cases drop and refuse the connection on purpose: Quickshell
  # warns of each socket error, and the client of the junk line it is sent.
  if [[ -f $(dirname -- "$log")/daemon.log || -d $(dirname -- "$log")/sock ]]; then
    problems=$(grep -vE 'quickshell\.io\.socket: Socket error|notchd client: ignoring a line|qmltest: the notchd daemon did not start|notchd-qmltest-daemon: ignoring ' <<< "$problems" || true)
  fi
  if [[ -n $problems ]]; then
    echo "QML warnings during the tests (shard $i):" >&2
    echo "$problems" >&2
    status=1
  fi
  grep -qE 'FAIL ' "$log" && status=1
  if ! grep -qE 'DONE$' "$log"; then
    echo "the QML tests did not finish (shard $i); log:" >&2
    cat "$log" >&2
    status=1
  fi
done

# What each case cost, for the next run's shards and scripts/test-times.py.
if [[ -z ${QMLTEST_ONLY:-} ]]; then
  mkdir -p -- "$(dirname -- "$times")"
  cat "$scratch"/*/log | sed -nE 's/.*TIME ([0-9]+) (.*)$/\1\t\2/p' \
    | awk -F'\t' '{ printf "%.3f\t%s\n", $1 / 1000, $2 }' | sort -rn > "$times.new"
  mv -- "$times.new" "$times"
fi
echo "qml: $(cat "$scratch"/*/log | grep -cE 'PASS ' || true) passed on $n shard(s)"
exit $status
