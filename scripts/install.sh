#!/bin/sh
# Installs the notchd daemon on a Linux distribution that is not Omarchy
# (Ubuntu, Fedora, ...): the pinned release, verified, plus a systemd user unit.
#
#   install.sh [--version V] [--prefix DIR] [--releases FILE|URL] [--no-service]
#              [--extension] [--uninstall]
#
# Needs sh, curl or wget, tar, sha256sum, and python3 (to read releases.json;
# Ubuntu and Fedora ship it). The checksums are the ones pinned in
# native/releases.json, never the release's own: from a checkout the script reads
# that file next to it, run alone (curl ... | sh) it reads the file on main.
#
#   --version V     the release to install; default: the newest one pinned
#   --prefix DIR    where `notchd` goes, DIR/bin/notchd; default ~/.local
#   --releases X    a releases.json file or URL (tests, mirrors)
#   --no-service    install the binary only: no unit, no systemctl
#   --extension     (later) install the GNOME Shell extension; not available yet
#   --uninstall     stop and remove the unit and the binary this script installed.
#                   ~/.config/notchd (your config) and the state are never touched.
#
# Run again to upgrade: the binary is replaced atomically and the daemon is
# restarted only when the binary or the unit changed. Only `notchd` is taken from
# the release's archive (any other file in it is ignored).
#
# The unit runs `notchd daemon` itself. Hosts that run `notchd daemon --ensure`
# (the Omarchy plugin, the standalone shell, the GNOME extension) find it
# running and start no second one. The daemon binds its own socket, so socket
# activation does not apply.
set -eu

die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }
say() { printf '%s\n' "$*"; }

prefix=${HOME:?HOME is not set}/.local
version=
releases=
service=1
extension=0
uninstall=0
default_releases=https://raw.githubusercontent.com/eryk-vieira/notchd/main/native/releases.json

while [ $# -gt 0 ]; do
  case $1 in
    --version) [ $# -ge 2 ] || die "--version needs a value"; version=$2; shift 2 ;;
    --prefix) [ $# -ge 2 ] || die "--prefix needs a value"; prefix=$2; shift 2 ;;
    --releases) [ $# -ge 2 ] || die "--releases needs a value"; releases=$2; shift 2 ;;
    --no-service) service=0; shift ;;
    --extension) extension=1; shift ;;
    --uninstall) uninstall=1; shift ;;
    -h|--help) sed -n '2,/^set -eu/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown option: $1 (see --help)" ;;
  esac
done

bin_dir=$prefix/bin
bin=$bin_dir/notchd
unit_dir=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
unit=$unit_dir/notchd.service

have() { command -v "$1" >/dev/null 2>&1; }

if [ "$uninstall" = 1 ]; then
  if [ -f "$unit" ]; then
    if have systemctl; then
      systemctl --user disable --now notchd.service >/dev/null 2>&1 || true
    fi
    rm -f "$unit"
    if have systemctl; then
      systemctl --user daemon-reload >/dev/null 2>&1 || true
    fi
    say "removed $unit"
  fi
  if [ -f "$bin" ]; then
    # A daemon a host started itself runs from this binary: stop it too.
    "$bin" ipc shutdown >/dev/null 2>&1 || true
    rm -f "$bin"
    say "removed $bin"
  fi
  # GNOME extension: removed here once it exists.
  say "kept your config (~/.config/notchd)"
  exit 0
fi

if [ "$extension" = 1 ]; then
  die "the GNOME Shell extension does not exist yet; --extension is reserved for it"
fi

for tool in tar sha256sum python3; do have "$tool" || die "$tool is required"; done
if have curl; then
  fetch() { curl -fsSL --retry 2 -o "$2" "$1"; }
elif have wget; then
  fetch() { wget -q -O "$2" "$1"; }
else
  die "curl or wget is required"
fi

case $(uname -s) in Linux) ;; *) die "this installs on Linux only" ;; esac
case $(uname -m) in
  x86_64|amd64) target=x86_64-unknown-linux-gnu ;;
  aarch64|arm64) target=aarch64-unknown-linux-gnu ;;
  *) die "no notchd build for $(uname -m)" ;;
esac
# Tests name the target of the fake release.
target=${NOTCHD_INSTALL_TARGET:-$target}

work=$(mktemp -d "${TMPDIR:-/tmp}/notchd-install.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM

if [ -z "$releases" ]; then
  here=$(cd -- "$(dirname -- "$0")" 2>/dev/null && pwd || true)
  if [ -n "$here" ] && [ -f "$here/../native/releases.json" ]; then
    releases=$here/../native/releases.json
  else
    releases=$default_releases
  fi
fi
case $releases in
  http://*|https://*|file://*)
    fetch "$releases" "$work/releases.json" || die "cannot download $releases"
    releases=$work/releases.json ;;
  *) [ -f "$releases" ] || die "no such file: $releases" ;;
esac

# "<version> <url> <sha256>" for the wanted version (the newest when none is asked).
entry=$(python3 -I - "$releases" "$version" "$target" <<'PY'
import json, re, sys
path, want, target = sys.argv[1:4]
try:
    data = json.load(open(path))
except (OSError, ValueError) as e:
    sys.exit("cannot read %s: %s" % (path, e))
def key(v):
    return [int(x) if x.isdigit() else 0 for x in re.split(r"[.-]", v)]
if not want:
    if not data:
        sys.exit("no release is pinned yet")
    want = max(data, key=key)
if want not in data:
    sys.exit("no release for version %s (pinned: %s)" % (want, ", ".join(sorted(data, key=key)) or "none"))
e = data[want].get(target)
if not e or not e.get("url") or not e.get("sha256"):
    sys.exit("version %s has no build for %s" % (want, target))
print(want, e["url"], e["sha256"].lower())
PY
) || die "$entry"
set -- $entry
version=$1
url=$2
sha=$3

installed=
if [ -x "$bin" ]; then
  installed=$("$bin" --version 2>/dev/null | awk '{print $NF}') || installed=
fi

changed=0
if [ "$installed" = "$version" ]; then
  say "notchd $version is already installed in $bin_dir"
else
  say "downloading notchd $version ($target)"
  fetch "$url" "$work/archive.tar.gz" || die "cannot download $url"
  got=$(sha256sum "$work/archive.tar.gz" | awk '{print $1}')
  [ "$got" = "$sha" ] || die "checksum mismatch for $url (got $got, pinned $sha): nothing installed"
  mkdir "$work/x"
  tar -xzf "$work/archive.tar.gz" -C "$work/x" || die "cannot unpack the archive"
  found=$(find "$work/x" -type f -name notchd | head -n 1)
  [ -n "$found" ] || die "the archive holds no notchd"
  mkdir -p "$bin_dir"
  cp "$found" "$bin_dir/.notchd.$$"
  chmod 755 "$bin_dir/.notchd.$$"
  if [ -f "$bin" ] && cmp -s "$bin" "$bin_dir/.notchd.$$"; then
    rm -f "$bin_dir/.notchd.$$"
  else
    mv -f "$bin_dir/.notchd.$$" "$bin"
    changed=1
  fi
  say "installed $bin"
fi

if [ "$service" = 1 ]; then
  mkdir -p "$unit_dir"
  new_unit=$work/notchd.service
  cat > "$new_unit" <<UNIT
[Unit]
Description=notchd daemon (config, modules, terminals and timers of the notch)
Documentation=https://github.com/eryk-vieira/notchd

[Service]
Type=simple
ExecStart=$bin daemon
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT
  unit_changed=0
  if [ ! -f "$unit" ] || ! cmp -s "$unit" "$new_unit"; then
    cp "$new_unit" "$unit"
    unit_changed=1
    say "installed $unit"
  fi
  if have systemctl; then
    if [ "$unit_changed" = 1 ]; then
      systemctl --user daemon-reload || say "systemctl --user daemon-reload failed"
    fi
    systemctl --user enable notchd.service >/dev/null 2>&1 ||
      say "could not enable notchd.service (no user session?): systemctl --user enable --now notchd.service"
    if [ "$changed" = 1 ] || [ "$unit_changed" = 1 ]; then
      # A running daemon (the unit's, or one a host started) has the old binary:
      # stop it (a clean exit, which the unit does not restart), then start ours.
      "$bin" ipc shutdown >/dev/null 2>&1 || true
      sleep 1
    fi
    systemctl --user start notchd.service || say "could not start notchd.service: see journalctl --user -u notchd"
  else
    say "no systemctl: run '$bin daemon --ensure' to start the daemon"
  fi
fi

# GNOME Shell extension: installed here once it exists (--extension).

case ":$PATH:" in *":$bin_dir:"*) ;; *) say "note: $bin_dir is not on your PATH" ;; esac
say "done"
