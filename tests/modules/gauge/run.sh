#!/bin/sh
# A user module for the QML tests (tests/qml/shell.qml, "a user module no
# notch lists is in the settings"): a setting, and a compact view once a notch
# lists it.
read -r init
printf '{"t":"set","active":true,"priority":20}\n'
printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"gauge on"}}\n'
while read -r line; do
  case "$line" in
    *'"t":"stop"'*) exit 0 ;;
  esac
done
