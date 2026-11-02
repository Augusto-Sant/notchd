#!/bin/sh
# Sixty views in a burst: the host keeps 20 a second and drops the rest.
read -r init
i=0
while [ $i -lt 60 ]; do
  printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"n%s"}}\n' "$i"
  i=$((i + 1))
done
while read -r line; do
  case "$line" in *'"t":"stop"'*) exit 0 ;; esac
done
