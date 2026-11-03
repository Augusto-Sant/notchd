#!/usr/bin/env bash
# Validates the plugin as it would be committed: `omarchy plugin validate` on a
# copy of the files git sees (tracked, and new ones not ignored), not on the
# checkout itself. The build folders are ignored and never ship (the plugin
# downloads the daemon), but they can hold symlinks the validator rejects: a
# daemon build keeps Ghostty's source and its
# libghostty-vt.so links under native/target/. The copy keeps symlinks as
# symlinks, so one added to the plugin still fails.
set -euo pipefail
cd "$(dirname "$0")/.."
copy=$(mktemp -d)
trap 'rm -rf -- "$copy"' EXIT
plugin="$copy/$(basename "$PWD")"
mkdir -p "$plugin"
# Files deleted from the checkout but still tracked are not copied.
git ls-files -z --cached --others --exclude-standard --deduplicate \
  | while IFS= read -r -d '' f; do
      if [[ -e $f || -L $f ]]; then printf '%s\0' "$f"; fi
    done \
  | xargs -0 -r cp --parents -P -t "$plugin"
omarchy plugin validate "$plugin"
