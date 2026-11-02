#!/bin/sh
# A user module for the QML tests (tests/qml/shell.qml, "a user module shows in
# the notch"): active with a compact view and an expanded one that has a
# button; the button's action changes the compact text.
say() {
  printf '{"t":"view","slot":"compact","tree":{"t":"row","gap":6,"children":[{"t":"icon","glyph":"H","tone":"accent"},{"t":"text","text":"%s"}]}}\n' "$1"
}
read -r init
printf '{"t":"set","active":true,"priority":40}\n'
say "hello there"
printf '{"t":"view","slot":"expanded","tree":{"t":"column","gap":8,"children":[{"t":"text","text":"Hello module","size":"heading"},{"t":"button","label":"Poke","action":"poke"}]}}\n'
while read -r line; do
  case "$line" in
    *'"t":"stop"'*) exit 0 ;;
    *'"action":"poke"'*) say "poked" ;;
  esac
done
