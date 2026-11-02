#!/usr/bin/env python3
"""A demo user module for `make gnome-dev` (scripts/gnome-dev.sh): one view
with the primitives the timer and resources modules do not use (image,
equalizer, stack, meter, list), so they can be looked at in a GNOME session
with no player and no microphone. Not a test, not shipped."""

import json
import math
import os
import select
import sys
import time

COVER = os.environ.get("NOTCHD_DEMO_COVER", "")


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def expanded(t, eq_on, eq_mode, lines):
    level = (math.sin(t * 3) + 1) / 2
    return {"t": "column", "gap": 12, "children": [
        {"t": "row", "gap": 12, "children": [
            {"t": "image", "src": COVER, "size": 58, "radius": "round",
             "fallback": {"t": "icon", "name": "audio-x-generic", "glyph": "\U000f075a", "tone": "muted"}},
            {"t": "column", "gap": 4, "fill": True, "children": [
                {"t": "text", "text": "Image, equalizer", "size": "heading", "bold": True},
                {"t": "row", "gap": 8, "children": [
                    {"t": "equalizer", "active": eq_on, "mode": eq_mode},
                    {"t": "text", "text": "%s, %s" % ("on" if eq_on else "off", eq_mode),
                     "tone": "muted", "size": "bodySmall"},
                ]},
            ]},
            {"t": "button", "name": "media-playback-pause" if eq_on else "media-playback-start",
             "glyph": "" if eq_on else "", "action": "eq"},
            {"t": "chip", "label": eq_mode, "action": "mode"},
        ]},
        {"t": "row", "gap": 12, "children": [
            {"t": "stack", "children": [
                {"t": "meter", "id": "voice", "value": level, "size": 48,
                 "child": {"t": "icon", "name": "audio-input-microphone", "glyph": "", "size": "heading"}},
            ]},
            {"t": "list", "id": "log", "gap": 4, "stick": "end", "maxHeight": 80, "fill": True,
             "children": [{"t": "text", "text": l, "size": "bodySmall", "tone": "muted"} for l in lines]},
            {"t": "image", "src": "/no/such/picture.png", "size": 38, "radius": "small",
             "fallback": {"t": "icon", "name": "dialog-warning", "glyph": "", "tone": "faint"}},
        ]},
    ]}


def main():
    send({"t": "set", "active": True})
    send({"t": "view", "slot": "compact", "tree": {"t": "row", "gap": 6, "children": [
        {"t": "icon", "name": "view-refresh", "glyph": "", "tone": "accent"},
        {"t": "text", "text": "primitives"}]}})
    eq_on, eq_mode, lines = True, "smooth", []
    start = last_line = time.time()
    while True:
        ready, _, _ = select.select([sys.stdin], [], [], 0.12)
        if ready:
            line = sys.stdin.readline()
            if not line:
                return
            msg = json.loads(line)
            if msg.get("t") == "stop":
                return
            if msg.get("t") == "event":
                if msg.get("action") == "eq":
                    eq_on = not eq_on
                elif msg.get("action") == "mode":
                    eq_mode = "light" if eq_mode == "smooth" else "smooth"
        now = time.time()
        if now - last_line > 1.5:
            last_line = now
            lines = (lines + ["line %d at %s" % (len(lines) + 1, time.strftime("%H:%M:%S"))])[-30:]
        send({"t": "view", "slot": "expanded", "tree": expanded(now - start, eq_on, eq_mode, lines)})


if __name__ == "__main__":
    main()
