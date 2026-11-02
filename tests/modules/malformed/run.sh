#!/bin/sh
# Garbage between good lines: not JSON, a cut-off object, a wrong slot, an
# unknown message type (ignored). The module must keep running.
read -r init
echo 'this is not json'
echo '{"t":"view"'
echo '{"t":"view","slot":"bogus","tree":null}'
echo '{"t":"nosuch","x":1}'
printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"still alive"}}\n'
while read -r line; do
  case "$line" in *'"t":"stop"'*) exit 0 ;; esac
done
