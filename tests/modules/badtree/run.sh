#!/bin/sh
# A view whose tree breaks a rule (progress.value 7 is outside 0..1); a valid
# one after any event.
read -r init
printf '{"t":"view","slot":"compact","tree":{"t":"progress","value":7}}\n'
while read -r line; do
  case "$line" in
    *'"t":"stop"'*) exit 0 ;;
    *'"t":"event"'*) printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"fixed"}}\n' ;;
  esac
done
