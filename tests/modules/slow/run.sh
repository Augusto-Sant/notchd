#!/bin/sh
# Takes two seconds to show anything.
read -r init
sleep 2
printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"finally"}}\n'
while read -r line; do
  case "$line" in *'"t":"stop"'*) exit 0 ;; esac
done
