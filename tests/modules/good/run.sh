#!/bin/sh
# A well-behaved module: one set, one compact view, a log line; answers an
# event with an expanded view; leaves on `stop` or when stdin closes.
read -r init
echo "$init" >&2
printf '{"t":"set","active":true,"priority":40}\n'
printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"hello"}}\n'
printf '{"t":"log","level":"info","text":"pid %s"}\n' "$$"
while read -r line; do
  case "$line" in
    *'"t":"stop"'*) exit 0 ;;
    *'"t":"event"'*) printf '{"t":"view","slot":"expanded","tree":{"t":"text","text":"clicked"}}\n' ;;
    *'"t":"config"'*) printf '{"t":"log","level":"info","text":"got config"}\n' ;;
  esac
done
