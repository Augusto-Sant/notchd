#!/usr/bin/env python3
"""A standalone module for tests/gnome_test.py: its own notch, which peeks
for 1.5 s every 3 s with a text. The GNOME UI must draw that guest notch
only while it peeks. Not shipped."""

import json
import select
import sys
import time


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def main():
    send({"t": "set", "active": True})
    send({"t": "view", "slot": "peek", "tree": {"t": "text", "text": "pinger peeks"}})
    next_peek = time.time() + 1
    while True:
        ready, _, _ = select.select([sys.stdin], [], [], max(0, next_peek - time.time()))
        if ready:
            line = sys.stdin.readline()
            if not line or json.loads(line).get("t") == "stop":
                return
            continue
        send({"t": "peek", "ms": 1500, "slot": "peek"})
        next_peek = time.time() + 3


if __name__ == "__main__":
    main()
