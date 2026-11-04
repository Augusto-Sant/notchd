#!/usr/bin/env python3
"""A view taller than a GNOME test monitor, to exercise expanded scrolling."""

import json
import sys


def send(message):
    print(json.dumps(message), flush=True)


send({"t": "set", "active": True})
send({"t": "view", "slot": "compact", "tree": {"t": "text", "text": "Tall view"}})
send({"t": "view", "slot": "expanded", "tree": {
    "t": "column", "gap": 10,
    "children": [{"t": "text", "text": "row %02d" % i} for i in range(70)],
}})
for line in sys.stdin:
    if json.loads(line).get("t") == "stop":
        break
