#!/bin/sh
# Installs the notchd daemon on a Linux distribution that is not Omarchy
# (Ubuntu, Fedora, ...): the pinned release, verified, plus a systemd user unit.
#
#   install.sh [--version V] [--prefix DIR] [--releases FILE|URL] [--binary FILE]
#              [--no-service] [--extension] [--extension-zip FILE] [--uninstall]
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
#   --binary FILE   install this notchd instead of a pinned release (a build of
#                   your own, before the first release); no download, no checksum
#   --extension     also install the GNOME Shell extension (GNOME 46 and later),
#                   built from this checkout (scripts/gnome-build.sh), into
#                   ~/.local/share/gnome-shell/extensions, and enable it; it
#                   loads at your next login. With no config yet it writes a
#                   starter one (~/.config/notchd/config.json: reserveSpace on),
#                   and says what to install for terminals in the notch (VTE
#                   for GTK 4) when it is missing
#   --extension-zip FILE  the extension from this zip (make gnome-zip) instead
#                   of the checkout; implies --extension
#   --uninstall     stop and remove the unit, the binary and the extension this
#                   script installed. ~/.config/notchd (your config) and the
#                   state are never touched.
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
extension_zip=
binary=
uninstall=0
uuid=notchd@eryk-vieira.github.io
default_releases=https://raw.githubusercontent.com/eryk-vieira/notchd/main/native/releases.json

while [ $# -gt 0 ]; do
  case $1 in
    --version) [ $# -ge 2 ] || die "--version needs a value"; version=$2; shift 2 ;;
    --prefix) [ $# -ge 2 ] || die "--prefix needs a value"; prefix=$2; shift 2 ;;
    --releases) [ $# -ge 2 ] || die "--releases needs a value"; releases=$2; shift 2 ;;
    --no-service) service=0; shift ;;
    --extension) extension=1; shift ;;
    --extension-zip) [ $# -ge 2 ] || die "--extension-zip needs a value"; extension=1; extension_zip=$2; shift 2 ;;
    --binary) [ $# -ge 2 ] || die "--binary needs a value"; binary=$2; shift 2 ;;
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

ext_dir=${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/$uuid
here=$(cd -- "$(dirname -- "$0")" 2>/dev/null && pwd || true)

# org.gnome.shell's enabled-extensions with the extension added (add) or
# taken out (remove), for when gnome-extensions cannot (no running shell).
enabled_list() {
  python3 -I - "$1" "$uuid" "$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || echo '[]')" <<'PY'
import ast, sys
op, uuid, current = sys.argv[1:4]
current = current.strip()
if current.startswith("@as"):
    current = current[3:].strip()
try:
    names = list(ast.literal_eval(current))
except (ValueError, SyntaxError):
    names = []
names = [n for n in names if n != uuid] + ([uuid] if op == "add" else [])
print("[" + ", ".join("'%s'" % n for n in names) + "]")
PY
}

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
  if [ -d "$ext_dir" ]; then
    if have gnome-extensions && gnome-extensions disable "$uuid" >/dev/null 2>&1; then :
    elif have gsettings; then gsettings set org.gnome.shell enabled-extensions "$(enabled_list remove)" >/dev/null 2>&1 || true
    fi
    rm -rf "$ext_dir"
    say "removed $ext_dir"
  fi
  say "kept your config (~/.config/notchd)"
  exit 0
fi

for tool in tar sha256sum python3; do have "$tool" || die "$tool is required"; done
if [ "$extension" = 1 ] && [ -z "$extension_zip" ] && ! [ -x "$here/gnome-build.sh" ]; then
  die "--extension needs a checkout of notchd (scripts/gnome-build.sh) or --extension-zip FILE"
fi
if [ -n "$extension_zip" ] && ! [ -f "$extension_zip" ]; then
  die "no such file: $extension_zip"
fi
if have curl; then
  fetch() { curl -fsSL --retry 2 -o "$2" "$1"; }
elif have wget; then
  fetch() { wget -q -O "$2" "$1"; }
else
  # Only a download needs one (--binary needs none).
  fetch() { die "curl or wget is required"; }
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

if [ -n "$binary" ]; then
  [ -f "$binary" ] || die "no such file: $binary"
  version=$("$binary" --version 2>/dev/null | awk '{print $NF}') || version=
  [ -n "$version" ] || die "$binary does not run as notchd (--version)"
  url=
  sha=
elif [ -z "$releases" ]; then
  if [ -n "$here" ] && [ -f "$here/../native/releases.json" ]; then
    releases=$here/../native/releases.json
  else
    releases=$default_releases
  fi
fi
if [ -z "$binary" ]; then
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
fi

installed=
if [ -x "$bin" ]; then
  installed=$("$bin" --version 2>/dev/null | awk '{print $NF}') || installed=
fi

changed=0
if [ -z "$binary" ] && [ "$installed" = "$version" ]; then
  say "notchd $version is already installed in $bin_dir"
else
  if [ -n "$binary" ]; then
    found=$binary
  else
    say "downloading notchd $version ($target)"
    fetch "$url" "$work/archive.tar.gz" || die "cannot download $url"
    got=$(sha256sum "$work/archive.tar.gz" | awk '{print $1}')
    [ "$got" = "$sha" ] || die "checksum mismatch for $url (got $got, pinned $sha): nothing installed"
    mkdir "$work/x"
    tar -xzf "$work/archive.tar.gz" -C "$work/x" || die "cannot unpack the archive"
    found=$(find "$work/x" -type f -name notchd | head -n 1)
    [ -n "$found" ] || die "the archive holds no notchd"
  fi
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
  # A user manager answers show-environment; a container or a session
  # without systemd has the command but nothing behind it.
  if have systemctl && systemctl --user show-environment >/dev/null 2>&1; then
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
    # No user manager: start the daemon detached, as a host would; it does
    # not come back at login, so say how to start it again.
    if [ "$changed" = 1 ]; then
      "$bin" ipc shutdown >/dev/null 2>&1 || true
      sleep 1
    fi
    if "$bin" daemon --ensure >/dev/null 2>&1; then
      say "no systemd user session: started the daemon with '$bin daemon --ensure' (run it again after a reboot)"
    else
      say "no systemd user session, and the daemon did not start: run '$bin daemon --ensure'"
    fi
  fi
fi

if [ "$extension" = 1 ]; then
  # The extension's folder: from the zip, else built from this checkout.
  mkdir "$work/ext"
  if [ -n "$extension_zip" ]; then
    python3 -I -m zipfile -e "$extension_zip" "$work/ext" || die "cannot unpack $extension_zip"
    src=$work/ext
  else
    src=$("$here/gnome-build.sh" "$work/ext") || die "cannot build the extension (scripts/gnome-build.sh)"
    rm -f "$src/testhook.js"
  fi
  [ -f "$src/metadata.json" ] || die "the extension has no metadata.json"
  mkdir -p "$(dirname -- "$ext_dir")"
  rm -rf "$ext_dir.new"
  cp -r "$src" "$ext_dir.new"
  rm -rf "$ext_dir"
  mv "$ext_dir.new" "$ext_dir"
  say "installed the GNOME Shell extension in $ext_dir"
  if have gnome-extensions && gnome-extensions enable "$uuid" >/dev/null 2>&1; then
    say "enabled it"
  elif have gsettings && gsettings set org.gnome.shell enabled-extensions "$(enabled_list add)" >/dev/null 2>&1; then
    say "enabled it"
  else
    say "enable it with: gnome-extensions enable $uuid"
  fi
  say "log out and back in to load it (GNOME loads a new extension at login)"
  # Terminals in the notch draw with VTE for GTK 4.
  if ! python3 -I -c "import gi; gi.require_version('Gtk', '4.0'); gi.require_version('Vte', '3.91')" >/dev/null 2>&1; then
    say "for terminals in the notch, install VTE for GTK 4:"
    say "  Ubuntu/Debian: sudo apt install gir1.2-vte-3.91    Fedora: sudo dnf install vte291-gtk4"
  fi
  # A starter config, only when there is none: the notch keeps windows clear
  # of itself.
  config=${XDG_CONFIG_HOME:-$HOME/.config}/notchd/config.json
  if [ ! -e "$config" ]; then
    mkdir -p "$(dirname -- "$config")"
    printf '{\n  "reserveSpace": true\n}\n' > "$config"
    say "wrote a starter config: $config"
  fi
fi

case ":$PATH:" in *":$bin_dir:"*) ;; *) say "note: $bin_dir is not on your PATH" ;; esac
say "done"
