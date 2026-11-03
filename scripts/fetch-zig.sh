#!/usr/bin/env bash
# Puts the zig that builds Ghostty's libghostty-vt (the daemon's terminal
# emulator, docs/architecture.md, "Terminals") in build/zig and prints its
# folder, for PATH. Ghostty wants exactly this version; a distribution's zig
# is often another. Downloaded once, its SHA-256 checked against the pins
# below (from ziglang.org's index), never run unchecked.
#
#   PATH="$(scripts/fetch-zig.sh):$PATH" cargo build ...
set -euo pipefail

version=0.15.2
case $(uname -m) in
  x86_64) arch=x86_64 sha=02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239 ;;
  aarch64 | arm64) arch=aarch64 sha=958ed7d1e00d0ea76590d27666efbf7a932281b3d7ba0c6b01b0ff26498f667f ;;
  *) echo "fetch-zig: no zig pinned for $(uname -m)" >&2; exit 1 ;;
esac

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
name=zig-$arch-linux-$version
dir=$root/build/zig/$name
if [[ -x $dir/zig ]]; then
  printf '%s\n' "$dir"
  exit 0
fi

mkdir -p -- "$root/build/zig"
tmp=$(mktemp -d "$root/build/zig/.fetch.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
echo "fetch-zig: downloading zig $version ($arch)" >&2
curl -sSfL --retry 3 -o "$tmp/$name.tar.xz" "https://ziglang.org/download/$version/$name.tar.xz"
echo "$sha  $tmp/$name.tar.xz" | sha256sum -c --quiet - >&2 \
  || { echo "fetch-zig: checksum mismatch for $name.tar.xz" >&2; exit 1; }
tar -C "$tmp" -xf "$tmp/$name.tar.xz"
rm -rf -- "$dir"
mv -- "$tmp/$name" "$dir"
printf '%s\n' "$dir"
