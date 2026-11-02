#!/bin/sh
# Counter: a number in the closed notch, and when it opens a button that adds
# one and a button that starts over. Plain shell, no libraries.
#
# What it shows:
#   - every line on stdout is one JSON message to notchd ("view", "set");
#   - every line on stdin is one from notchd ("init", "event", "stop"...);
#   - "${cache}" in module.json is a folder of this module's own; the count
#     is kept there, so it survives a restart.
state="$1/count"
count=$(cat "$state" 2>/dev/null || echo 0)

show() {
  printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"%s","tabular":true}}\n' "$count"
  printf '{"t":"view","slot":"expanded","tree":{"t":"row","gap":10,"align":"center","children":[{"t":"text","text":"Count: %s","size":"heading"},{"t":"spacer","fill":true},{"t":"button","label":"Add one","action":"add"},{"t":"button","label":"Reset","action":"reset","variant":"outline"}]}}\n' "$count"
  echo "$count" > "$state"
}

printf '{"t":"set","active":true}\n'
show

while read -r line; do
  case "$line" in
    *'"t":"stop"'*) exit 0 ;;
    *'"action":"add"'*) count=$((count + 1)); show ;;
    *'"action":"reset"'*) count=0; show ;;
  esac
done
