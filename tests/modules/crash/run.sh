#!/bin/sh
# Reads init, complains on stderr, exits 3.
read -r init
echo "boom: something broke" >&2
exit 3
