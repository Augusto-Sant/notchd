#!/bin/sh
# Nothing but noise on stdout: ten bad lines in a row count as a crash.
read -r init
i=0
while [ $i -lt 30 ]; do
  echo "noise $i"
  i=$((i + 1))
done
while read -r line; do :; done
