#!/usr/bin/env bash
# A GNOME session in a window of the desktop you are on (`gnome-shell
# --devkit`), with the notchd extension enabled and a daemon of its own:
# a private D-Bus, its own XDG_RUNTIME_DIR, config, data and cache dirs and
# notchd socket, so nothing touches the real session, the real daemon or
# ~/.config/notchd. Everything goes when the window closes.
#
#   scripts/gnome-dev.sh [--headless | --nested] [-- extra gnome-shell args]
#
# --nested: GNOME 46 to 48 have no devkit; `--nested` draws the session in
# an X11 window instead (Xwayland on a Wayland desktop: DISPLAY is kept).
#
# NOTCHD_BIN picks the daemon (default: native/target/release/notchd).
# For tests: NOTCHD_GNOME_SCRATCH (an empty, short dir to use and leave in
# place), NOTCHD_GNOME_CONFIG (the daemon's config file to start from) and
# NOTCHD_GNOME_MODULES (a folder of user modules for the daemon's modules
# dir, with a config; the demo ones go there when no config is given) and
# NOTCHD_GNOME_TEST (the extension exports its test hooks, testhook.js). The
# private bus's address is written to <scratch>/bus once the shell starts.
# The scratch dir is short on purpose: a Unix socket path is at most 108
# bytes, and mutter puts its Wayland socket in XDG_RUNTIME_DIR.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
headless=0
nested=0
extra=()
while [ $# -gt 0 ]; do
  case $1 in
    --headless) headless=1 ;;
    --nested) nested=1 ;;
    --) shift; extra=("$@"); break ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

bin=${NOTCHD_BIN:-$root/native/target/release/notchd}
[ -x "$bin" ] || { echo "no daemon at $bin: run make daemon" >&2; exit 1; }

keep=0
if [ -n "${NOTCHD_GNOME_SCRATCH:-}" ]; then
  scratch=$NOTCHD_GNOME_SCRATCH
  keep=1
else
  scratch=$(mktemp -d /tmp/ngd.XXXXXX)
fi
mkdir -p "$scratch"/{run,config,data,cache}
chmod 700 "$scratch/run"
daemon_pid=""
cleanup() {
  [ -n "$daemon_pid" ] && kill "$daemon_pid" 2>/dev/null || true
  pkill -f -- "--socket $scratch/" 2>/dev/null || true
  # The document portal and gvfs mount FUSE filesystems in the runtime dir.
  for m in $(awk -v d="$scratch/" 'index($2, d) == 1 {print $2}' /proc/mounts); do
    fusermount3 -u -z "$m" 2>/dev/null || fusermount -u -z "$m" 2>/dev/null || true
  done
  [ $keep -eq 1 ] || rm -rf -- "$scratch"
}
trap cleanup EXIT

uuid=$("$root/scripts/gnome-build.sh" "$scratch/data/gnome-shell/extensions" | xargs basename)

# A demo config: one notch with modules that need nothing installed.
if [ -n "${NOTCHD_GNOME_CONFIG:-}" ]; then
  cp -- "$NOTCHD_GNOME_CONFIG" "$scratch/notchd.json"
  [ -z "${NOTCHD_GNOME_MODULES:-}" ] || cp -r -- "$NOTCHD_GNOME_MODULES" "$scratch/modules"
else
  cat > "$scratch/notchd.json" <<'EOF'
{"modules": ["activities", "timer", "resources", "primitives"]}
EOF
  # A demo user module that draws the primitives the others do not use.
  cp -r -- "$root/tests/gnome/modules" "$scratch/modules"
fi
socket="$scratch/notchd.sock"
# XDG_STATE_HOME: user module logs stay in the scratch dir.
XDG_RUNTIME_DIR="$scratch/run" XDG_STATE_HOME="$scratch/state" NOTCHD_OFFLINE=1 \
  NOTCHD_DEMO_COVER="$root/assets/icons/hicolor/256x256/apps/notchd.png" \
  "$bin" --socket "$socket" --config "$scratch/notchd.json" daemon > "$scratch/daemon.log" 2>&1 &
daemon_pid=$!

host_wayland=""
host_runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
pipewire=()
if [ $headless -eq 0 ] && [ $nested -eq 0 ]; then
  # The devkit window shows the session through PipeWire: the host's.
  pipewire=(PIPEWIRE_RUNTIME_DIR="$host_runtime")
  [ -n "${WAYLAND_DISPLAY:-}" ] || { echo "no WAYLAND_DISPLAY: run it inside a Wayland session, or --headless" >&2; exit 1; }
  case $WAYLAND_DISPLAY in
    /*) host_wayland=$WAYLAND_DISPLAY ;;
    *) host_wayland=$host_runtime/$WAYLAND_DISPLAY ;;
  esac
fi

echo "scratch: $scratch (daemon log: $scratch/daemon.log)"
echo "try: NOTCHD_SOCKET=$socket $bin ipc push '{\"id\":\"demo\",\"title\":\"Deploying api\",\"progress\":40,\"sticky\":true}'"

# The windows you look at animate even where GNOME would turn animations
# off (the 46 container renders in software); the headless tests do not.
mode=(--devkit --force-animations)
[ $headless -eq 1 ] && mode=(--headless --virtual-monitor 1920x1080)
keep_display=()
if [ $nested -eq 1 ]; then
  [ -n "${DISPLAY:-}" ] || { echo "--nested needs DISPLAY (Xwayland)" >&2; exit 1; }
  mode=(--nested --force-animations)
  keep_display=(DISPLAY="$DISPLAY")
fi

env -u DISPLAY -u HYPRLAND_INSTANCE_SIGNATURE -u DBUS_SESSION_BUS_ADDRESS \
  -u QT_QPA_PLATFORMTHEME -u GTK_THEME \
  XDG_CURRENT_DESKTOP=GNOME XDG_SESSION_DESKTOP=gnome XDG_SESSION_TYPE=wayland \
  XDG_RUNTIME_DIR="$scratch/run" XDG_CONFIG_HOME="$scratch/config" \
  XDG_DATA_HOME="$scratch/data" XDG_CACHE_HOME="$scratch/cache" \
  WAYLAND_DISPLAY="$host_wayland" NO_AT_BRIDGE=1 "${pipewire[@]}" "${keep_display[@]}" \
  NOTCHD_SOCKET="$socket" NOTCHD_GNOME_BUS_FILE="$scratch/bus" SHELL_DEBUG=backtrace-warnings \
  dbus-run-session -- bash -c '
    printf "%s\n" "$DBUS_SESSION_BUS_ADDRESS" > "$NOTCHD_GNOME_BUS_FILE"
    gsettings set org.gnome.shell enabled-extensions "[\"$0\"]"
    gsettings set org.gnome.shell disable-user-extensions false
    gsettings set org.gnome.shell welcome-dialog-last-shown-version "999"
    if [ -n "${NOTCHD_GNOME_TEST:-}" ]; then
      gsettings set org.gnome.desktop.notifications show-banners false
      gsettings set org.gnome.desktop.interface enable-hot-corners false
      # A plain background: the shell waits for the wallpaper to load before
      # it finishes starting, and a minimal system may have no loader for it.
      gsettings set org.gnome.desktop.background picture-options none
    fi
    exec gnome-shell --wayland --no-x11 "$@"
  ' "$uuid" "${mode[@]}" "${extra[@]}"
