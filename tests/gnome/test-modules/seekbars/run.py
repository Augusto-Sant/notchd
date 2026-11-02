#!/usr/bin/env python3
"""A module for tests/gnome_test.py: the progress bars (an anchored,
seekable one, a vertical one) and the pictures (from the URLs the test's
server gives in the environment) the GNOME UI has to draw. It tells what
`seek` events reach it in a text, and does not move the bar itself: only
the anchor does. Not shipped."""

import json
import os
import sys
import time

FALLBACK = {"t": "icon", "name": "audio-x-generic", "glyph": "\U000f075a", "tone": "muted"}
START_MS = int(time.time() * 1000)


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def tree(seen, value):
    return {"t": "column", "gap": 8, "children": [
        {"t": "text", "text": seen},
        {"t": "progress", "id": "pos", "value": value, "seek": "seek", "tone": "accent",
         "anchor": {"at": START_MS, "rate": 0.05}},
        {"t": "row", "gap": 8, "children": [
            {"t": "progress", "id": "vol", "orientation": "v", "value": 0.5, "seek": "vseek", "thickness": 6},
            {"t": "progress", "id": "still", "value": 0.4},
            {"t": "image", "src": os.environ.get("NOTCHD_TEST_COVER", ""), "size": 58, "radius": "round", "fallback": FALLBACK},
            {"t": "image", "src": os.environ.get("NOTCHD_TEST_MISSING", ""), "size": 38, "radius": "small", "fallback": FALLBACK},
        ]},
    ]}


def main():
    send({"t": "set", "active": True})
    send({"t": "view", "slot": "compact", "tree": {"t": "text", "text": "seekbars"}})
    seen, value = "seek: none", 0.1
    while True:
        send({"t": "view", "slot": "expanded", "tree": tree(seen, value)})
        line = sys.stdin.readline()
        if not line:
            return
        msg = json.loads(line)
        if msg.get("t") == "stop":
            return
        if msg.get("t") == "event":
            seen = "%s: %.2f" % (msg.get("action"), msg.get("value") or 0)


if __name__ == "__main__":
    main()
