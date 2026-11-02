#!/usr/bin/env python3
"""CPU and memory in the notch, from notchd's `resources` provider.

What it shows:
  - "subscribe": ["resources"] in module.json: notchd then sends a
    {"t": "provider", "name": "resources", "data": {...}} line about once a second;
  - a view with progress bars, built as plain dicts;
  - anything printed to stderr (or sent as a "log" message) goes to the module's log.
"""
import json
import sys


def send(**message):
    print(json.dumps(message), flush=True)


def bar(label, fraction):
    return {
        "t": "row", "gap": 8, "align": "center", "children": [
            {"t": "text", "text": label, "tone": "muted", "width": 40},
            {"t": "progress", "value": max(0.0, min(1.0, fraction)), "fill": True,
             "tone": "danger" if fraction > 0.9 else "accent"},
            {"t": "text", "text": f"{fraction * 100:.0f}%", "tabular": True, "width": 40},
        ]}


def show(data):
    cpu = data["cpu"]
    mem = data["mem"]["used"] / max(1, data["mem"]["total"])
    send(t="view", slot="compact", tree={"t": "text", "text": f"{cpu * 100:.0f}%", "tabular": True})
    send(t="view", slot="expanded",
         tree={"t": "column", "gap": 8, "children": [bar("CPU", cpu), bar("RAM", mem)]})


send(t="set", active=True)
send(t="log", level="info", text="waiting for the resources provider")
for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue
    kind = message.get("t")
    if kind == "provider" and message.get("name") == "resources":
        show(message["data"])
    elif kind == "stop":
        break
