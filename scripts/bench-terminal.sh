#!/usr/bin/env bash
# Measures what a terminal in the notch costs: the same terminal (760x420),
# drawn by core/terminal/GhosttySurface.qml from the screen the daemon
# emulates (libghostty-vt), in the standalone host on your real Wayland
# session, as scripts/bench.sh runs it (a private D-Bus that activates
# nothing, a short XDG_RUNTIME_DIR of its own, so its own daemon and a scratch
# config). The program in the terminal (bench-terminal-driver.py) goes through
# three phases:
#
#   idle    the terminal open, nothing printed (BENCH_SECONDS)
#   flood   FLOOD_MB of colored lines, as fast as the terminal takes them:
#           its time is the end-to-end throughput (the PTY waits for the view)
#   anim    the whole screen redrawn 30 times a second (ANIM_SECONDS), as a TUI
#
# and each process is sampled from /proc over each phase: CPU (% of one core,
# and CPU seconds for the flood), context switches per second, PSS: quickshell
# (the view) and the daemon (the PTY and the emulator). The numbers it was
# chosen on, against qmltermwidget, are in the commit that added it. A notch
# shows on your screen while it runs (about a minute); keep the pointer away
# and the machine quiet: a click elsewhere closes it, and the phases it was
# closed in are marked not valid.
#
#   scripts/bench-terminal.sh [out]       # out: build/bench-terminal.md
#   NOTCHD_BIN=<daemon built with the ghostty feature> (default native/target/release/notchd)
set -euo pipefail

if [[ ${1:-} == --inner ]]; then
  shift
  root=$1 scratch=$2 renderer=$3 secs=$4 runtime=$5
  cfg=$scratch/config-$renderer.json
  phases=$scratch/phases-$renderer
  mkdir -p "$phases"
  rm -f "$phases"/*
  python3 - "$cfg" "$root/scripts/bench-terminal-driver.py" "$phases" <<'EOF'
import json, sys
cfg, driver, phases = sys.argv[1:]
json.dump({"modules": ["bench"],
           "terminals": {"bench": {"title": "Bench", "command": f"exec python3 {driver} {phases}"}},
           "bench": {"width": 760, "height": 420}}, open(cfg, "w"))
EOF
  NOTCHD_CONFIG=$cfg quickshell -p "$root" --no-color > "$scratch/qs-$renderer.log" 2>&1 &
  qs_pid=$!
  for _ in $(seq 75); do
    qs -p "$root" ipc call notchd state >/dev/null 2>&1 && break
    sleep 0.2
  done
  sleep 2
  qs -p "$root" ipc call notchd terminal bench toggle >/dev/null
  for _ in $(seq 150); do [[ -e $phases/ready ]] && break; sleep 0.1; done
  [[ -e $phases/ready ]] || { echo "the terminal did not start ($renderer)" >&2; tail -20 "$scratch/qs-$renderer.log" >&2; exit 1; }
  sleep 3

  # Quickshell runs the shell in a child of the process started here (the
  # parent only watches it for crashes): that child is the view.
  view_pid=$(pgrep -P "$qs_pid" -x quickshell | head -1 || true)
  daemon_pid=$(pgrep -f -- "--socket $runtime/.* daemon" | head -1 || true)
  attach_pid=$(pgrep -f -- "--socket $runtime/.* attach term:bench" | head -1 || true)
  pids=("${view_pid:-$qs_pid}" "${daemon_pid:-}" "${attach_pid:-}")
  echo "$renderer: view $view_pid (watcher $qs_pid), daemon $daemon_pid, attach ${attach_pid:-none}" >> "$scratch/pids"
  hz=$(getconf CLK_TCK)
  ticks() { [[ -n $1 && -r /proc/$1/stat ]] && sed 's/.*) //' "/proc/$1/stat" | awk '{ print $12 + $13 }' || echo 0; }
  ctx() { [[ -n $1 && -d /proc/$1 ]] && cat /proc/"$1"/task/*/status 2>/dev/null | awk '/ctxt_switches/ { s += $2 } END { print s + 0 }' || echo 0; }
  pss() { [[ -n $1 && -r /proc/$1/smaps_rollup ]] && awk '/^Pss:/ { printf "%.0f", $2 / 1024 }' "/proc/$1/smaps_rollup" || echo "-"; }
  # phase <name> <until-file or ""> : samples every process from now until
  # the file appears (or for `secs`).
  phase() {
    local name=$1 until=$2 t0 t1 c0=() c1=() x0=() x1=() i last=""
    qs -p "$root" ipc call notchd state > "$scratch/state-$renderer-$name.json" 2>&1 || true
    pgrep -af -- "--socket $runtime/" > "$scratch/procs-$renderer-$name.txt" || true
    t0=$(date +%s.%N)
    for i in 0 1 2; do c0[i]=$(ticks "${pids[i]}"); x0[i]=$(ctx "${pids[i]}"); done
    if [[ -n $until ]]; then
      touch "$phases/go-$name"
      while [[ ! -e $phases/$until ]]; do sleep 0.05; done
      # The program is done, but the view may not be: what is still queued
      # between them gets drawn. Until the three are quiet (under 5% of a
      # core over 0.3 s, three times running); the phase ends at the last
      # busy moment.
      local quiet=0 busy_at prev now
      prev=$(( $(ticks "${pids[0]}") + $(ticks "${pids[1]}") + $(ticks "${pids[2]}") ))
      busy_at=$(date +%s.%N)
      while (( quiet < 3 )); do
        sleep 0.3
        now=$(( $(ticks "${pids[0]}") + $(ticks "${pids[1]}") + $(ticks "${pids[2]}") ))
        if (( (now - prev) * 1000 / hz > 15 )); then quiet=0; busy_at=$(date +%s.%N); else quiet=$((quiet + 1)); fi
        prev=$now
      done
      last=$busy_at
    else
      sleep "$secs"
    fi
    t1=${last:-$(date +%s.%N)}
    for i in 0 1 2; do c1[i]=$(ticks "${pids[i]}"); x1[i]=$(ctx "${pids[i]}"); done
    local extra=""
    # A click elsewhere closes the notch, and a closed notch draws nothing:
    # such a phase measures nothing (nor does one whose `notchd attach` went).
    local invalid=""
    qs -p "$root" ipc call notchd state 2>/dev/null | grep -q '"mode": *"expanded"' || invalid=1
    [[ -n ${pids[2]} && ! -d /proc/${pids[2]} ]] && invalid=1
    [[ $name == flood ]] && extra="program $(cat "$phases/flood-done") s, drawn $(awk -v a="$t0" -v b="$t1" 'BEGIN { printf "%.2f", b - a }') s"
    [[ $name == anim ]] && extra="$(cat "$phases/anim-done") frames"
    [[ -n $invalid ]] && extra="$extra **(the notch closed: not valid)**"
    awk -v r="$renderer" -v n="$name" -v t0="$t0" -v t1="$t1" -v hz="$hz" -v extra="$extra" \
        -v q0="${c0[0]}" -v q1="${c1[0]}" -v d0="${c0[1]}" -v d1="${c1[1]}" -v a0="${c0[2]}" -v a1="${c1[2]}" \
        -v qx0="${x0[0]}" -v qx1="${x1[0]}" -v dx0="${x0[1]}" -v dx1="${x1[1]}" \
        -v qp="$(pss "${pids[0]}")" -v dp="$(pss "${pids[1]}")" -v has_attach="${pids[2]:+1}" \
        'BEGIN { dt = t1 - t0
                 q = (q1 - q0) / hz; d = (d1 - d0) / hz; a = (a1 - a0) / hz
                 att = has_attach ? sprintf("%.1f", a * 100 / dt) : "-"
                 # A flood the view did not draw is no measure of it.
                 if (n == "flood" && q * 100 / dt < 2) extra = extra " **(the view drew nothing: not valid)**"
                 printf "| %s | %s | %s | %.1f | %.1f | %s | **%.1f** | %.2f | %.0f | %.0f | %s | %s |\n",
                        r, n, extra, q * 100 / dt, d * 100 / dt, att, (q + d + a) * 100 / dt, q + d + a,
                        (qx1 - qx0) / dt, (dx1 - dx0) / dt, qp, dp }' >> "$scratch/results"
  }
  phase idle ""
  phase flood flood-done
  sleep 2
  phase anim anim-done
  qs -p "$root" ipc call notchd close >/dev/null 2>&1 || true
  kill "$qs_pid" 2>/dev/null || true
  wait "$qs_pid" 2>/dev/null || true
  [[ -n $daemon_pid ]] && kill "$daemon_pid" 2>/dev/null || true
  pkill -f -- "--socket $runtime/" 2>/dev/null || true
  exit 0
fi

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/build/bench-terminal.md"}
secs=${BENCH_SECONDS:-10}
export NOTCHD_BIN=${NOTCHD_BIN:-$root/native/target/release/notchd}
export FLOOD_MB=${FLOOD_MB:-200} ANIM_SECONDS=${ANIM_SECONDS:-10}

for tool in quickshell qs dbus-run-session python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool is required" >&2; exit 1; }
done
[[ -n ${WAYLAND_DISPLAY:-} ]] || { echo "needs a Wayland session (WAYLAND_DISPLAY is unset)" >&2; exit 1; }
[[ -x $NOTCHD_BIN ]] || { echo "no daemon at $NOTCHD_BIN" >&2; exit 1; }
session_runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}

scratch=$(mktemp -d "${TMPDIR:-/tmp}/notchd-bench-term.XXXXXX")
runtime=$(mktemp -d /tmp/nbt.XXXXXX)
cleanup() {
  pkill -f -- "--socket $runtime/" 2>/dev/null || true
  pkill -P $$ 2>/dev/null || true
  # BENCH_KEEP=1 keeps the scratch dir (logs, the notch's state per phase).
  [[ -n ${BENCH_KEEP:-} ]] && echo "kept: $scratch" >&2 || rm -rf -- "$scratch"
  rm -rf -- "$runtime"
}
trap cleanup EXIT
chmod 700 -- "$runtime"
case $WAYLAND_DISPLAY in
  /*) ;;
  *) ln -s -- "$session_runtime/$WAYLAND_DISPLAY" "$runtime/$WAYLAND_DISPLAY" ;;
esac
[[ -d $session_runtime/hypr ]] && ln -s -- "$session_runtime/hypr" "$runtime/hypr"

cat > "$scratch/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir=$runtime</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
: > "$scratch/results"

for renderer in terminal; do
  echo "bench-terminal: $renderer" >&2
  env -u DBUS_SESSION_BUS_ADDRESS -u QT_QPA_PLATFORMTHEME \
    XDG_RUNTIME_DIR="$runtime" NO_AT_BRIDGE=1 \
    dbus-run-session --config-file="$scratch/bus.conf" -- \
    bash "$root/scripts/bench-terminal.sh" --inner "$root" "$scratch" "$renderer" "$secs" "$runtime" \
    >> "$scratch/bus.log" 2>&1 || { echo "the $renderer run failed:" >&2; tail -20 "$scratch/bus.log" >&2; exit 1; }
done

{
  echo "# notchd terminal"
  echo
  echo "$(date -Iseconds) · $(git -C "$root" describe --always --dirty 2>/dev/null || echo unknown)" \
    "· idle ${secs}s, flood ${FLOOD_MB} MB, anim ${ANIM_SECONDS}s at 30 fps · terminal 760x420"
  echo
  echo "CPU in % of one core over the phase; CPU s is the three processes' CPU time in it."
  echo
  echo "| | Phase | Result | quickshell % | daemon % | attach % | total % | CPU s | qs ctx/s | daemon ctx/s | qs PSS MB | daemon PSS MB |"
  echo "|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
  cat "$scratch/results"
} > "$scratch/report"
mkdir -p -- "$(dirname -- "$out")"
cp -- "$scratch/report" "$out"
cat "$out"
