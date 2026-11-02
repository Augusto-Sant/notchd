#!/bin/sh
# Ignores SIGTERM and `stop`, and leaves a background child: only SIGKILL of
# the whole process group ends it.
trap '' TERM
sleep 300 &
printf '{"t":"log","level":"info","text":"pids %s %s"}\n' "$$" "$!"
printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"stubborn"}}\n'
while :; do
  sleep 0.1
done
