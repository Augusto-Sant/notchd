#!/usr/bin/env python3
"""Show a local Herdr session through notchd's user-module protocol.

Herdr owns the agents. This process only reads its snapshot and event stream;
it never starts an agent or modifies a Herdr session.
"""

import json
import os
from pathlib import Path
import selectors
import socket
import subprocess
import sys
import time


ORDER = {"blocked": 0, "done": 1, "working": 2, "idle": 3, "unknown": 4}
DEFAULTS = {"session": "", "interval": 2, "blocked": True, "done": True,
            "peekMs": 5000, "desktop": False}
LIFECYCLE = ("pane.created", "pane.closed", "pane.agent_detected", "pane.moved",
             "workspace.renamed", "tab.renamed")
ICON = str(Path(__file__).resolve().with_name("herdr.svg"))


def send(**message):
    sys.stdout.write(json.dumps(message, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def txt(value, tone="foreground", size="bodySmall", **more):
    return {"t": "text", "text": str(value), "tone": tone, "size": size, **more}


def col(children, gap=5):
    return {"t": "column", "gap": gap, "children": children}


def row(children, gap=8):
    return {"t": "row", "gap": gap, "align": "center", "children": children}


def logo(size):
    return {"t": "image", "src": ICON, "size": size, "fit": "contain",
            "fallback": {"t": "icon", "glyph": "󰚩", "tone": "accent"}}


def socket_path(session):
    if session:
        root = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
        return root / "herdr" / "sessions" / session / "herdr.sock"
    if os.environ.get("HERDR_SOCKET_PATH"):
        return Path(os.environ["HERDR_SOCKET_PATH"])
    inherited = os.environ.get("HERDR_SESSION", "")
    return socket_path(inherited) if inherited else Path(
        os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config") / "herdr" / "herdr.sock"


def snapshot(session):
    cmd = ["herdr"]
    if session:
        cmd += ["--session", session]
    result = subprocess.run(cmd + ["api", "snapshot"], capture_output=True,
                            text=True, timeout=4, check=True)
    data = json.loads(result.stdout)
    return data["result"]["snapshot"]


def agents_of(snap):
    spaces = {w["workspace_id"]: w.get("label") or w["workspace_id"]
              for w in snap.get("workspaces", [])}
    agents = {}
    for agent in snap.get("agents", []):
        pane = agent.get("pane_id")
        if not pane:
            continue
        agents[pane] = {
            "pane": pane,
            "agent": agent.get("display_agent") or agent.get("name") or agent.get("agent") or "Agent",
            "state": agent.get("agent_status") or "unknown",
            "title": agent.get("terminal_title_stripped") or agent.get("title") or "",
            "where": spaces.get(agent.get("workspace_id"), agent.get("workspace_id") or ""),
            "seq": agent.get("state_change_seq") or 0,
        }
    return agents


def pane_ids(snap):
    return tuple(sorted(p["pane_id"] for p in snap.get("panes", []) if p.get("pane_id")))


def notices(before, after, config):
    """Only new attention or completion states count; bootstrap has no `before`."""
    if before is None:
        return []
    changed = []
    for pane, agent in after.items():
        previous = before.get(pane)
        state = agent["state"]
        attention = state in ("blocked", "done") and config.get(state, True)
        finished = (state == "idle" and previous is not None
                    and previous["state"] == "working" and config.get("done", True))
        if previous and previous["state"] != state and (attention or finished):
            changed.append(agent)
    return sorted(changed, key=lambda a: (ORDER.get(a["state"], 9), -a["seq"]))


class Module:
    def __init__(self):
        self.config = DEFAULTS.copy()
        self.selector = selectors.DefaultSelector()
        self.selector.register(sys.stdin, selectors.EVENT_READ, "stdin")
        self.stdin_buffer = b""
        self.connection = None
        self.socket_buffer = b""
        self.subscribed_panes = ()
        self.agents = None
        self.workspaces = 0
        self.panes = 0
        self.version = ""
        self.up = False
        self.rendered = False
        self.next_poll = 0

    def close_connection(self):
        if self.connection is not None:
            try:
                self.selector.unregister(self.connection)
            except (KeyError, ValueError):
                pass
            self.connection.close()
            self.connection = None
        self.socket_buffer = b""
        self.subscribed_panes = ()

    def subscribe(self, panes):
        if self.connection is not None and panes == self.subscribed_panes:
            return
        self.close_connection()
        subscriptions = [{"type": kind} for kind in LIFECYCLE]
        subscriptions += [{"type": "pane.agent_status_changed", "pane_id": pane}
                          for pane in panes]
        conn = None
        try:
            conn = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            conn.settimeout(2)
            conn.connect(str(socket_path(self.config["session"])))
            request = {"id": "notchd-herdr", "method": "events.subscribe",
                       "params": {"subscriptions": subscriptions}}
            conn.sendall((json.dumps(request) + "\n").encode())
            conn.setblocking(False)
            self.selector.register(conn, selectors.EVENT_READ, "herdr")
            self.connection = conn
            self.subscribed_panes = panes
        except (OSError, ValueError):
            if conn is not None:
                conn.close()

    def render(self):
        self.rendered = True
        if not self.up:
            send(t="set", active=True, attention=False, urgentPeek=False)
            send(t="view", slot="compact", tree=row([
                logo(15), txt("Herdr", "accent"), txt("Offline", "muted")], 6))
            send(t="view", slot="expanded", tree=col([
                row([logo(22), txt("Herdr", "foreground", "heading", bold=True)], 8),
                txt("Herdr is not running", "muted")], 8))
            return
        agents = sorted(self.agents.values(), key=lambda a:
                        (ORDER.get(a["state"], 9), -a["seq"], a["pane"]))
        counts = {state: sum(a["state"] == state for a in agents)
                  for state in ("blocked", "done", "working", "idle")}
        needs_you = counts["blocked"] or counts["done"]
        send(t="set", active=True, attention=bool(needs_you),
             urgentPeek=bool(counts["blocked"]))
        summary = (f'{counts["blocked"] + counts["done"]} need you'
                   if needs_you else f'{counts["working"]} working')
        send(t="view", slot="compact", tree=row([
            logo(15), txt("Herdr", "accent"), txt(summary, "foreground")], 6))
        def metric(state, tone):
            return col([
                txt(counts[state], tone, "heading", bold=True, tabular=True),
                txt(state.capitalize(), "muted", "caption")], 2)

        header = row([
            logo(22),
            txt("Herdr", "foreground", "heading", bold=True),
            {"t": "spacer", "fill": True},
            txt(f'v{self.version}', "muted", "caption") if self.version else txt("", "muted")], 8)
        scope = txt(f'{len(agents)} agents  ·  {self.workspaces} workspaces  ·  '
                    f'{self.panes} panes', "muted", "bodySmall")
        metrics = row([metric("working", "accent"), {"t": "spacer", "fill": True},
                       metric("blocked", "danger"), {"t": "spacer", "fill": True},
                       metric("done", "accent"), {"t": "spacer", "fill": True},
                       metric("idle", "foreground")], 16)
        send(t="view", slot="expanded", tree=col([header, scope, metrics], 14))
        if needs_you:
            agent = agents[0]
            send(t="view", slot="peek", tree=col([
                txt(f'{agent["agent"]}: {agent["state"]}',
                    "danger" if agent["state"] == "blocked" else "accent", "body"),
                txt(agent["title"] or agent["where"], "muted", "caption")]))

    def announce(self, events):
        first = events[0]
        state = first["state"]
        if len(events) == 1:
            verb = "needs attention" if state == "blocked" else "finished"
            title = f'{first["agent"]} {verb}'
            body = f'{first["title"] or first["where"]} · {first["where"]}'
        else:
            title = f'{len(events)} Herdr agents changed state'
            body = ", ".join(f'{a["agent"]}: '
                             f'{"finished" if a["state"] == "idle" else a["state"]}'
                             for a in events[:4])
            if len(events) > 4:
                body += f', and {len(events) - 4} more'
        send(t="view", slot="peek", tree=col([
            txt(title, "danger" if state == "blocked" else "accent", "body"),
            txt(body, "muted", "caption")]))
        try:
            duration = max(400, min(30000, int(self.config["peekMs"])))
        except (TypeError, ValueError):
            duration = 5000
        send(t="peek", ms=duration)
        if self.config.get("desktop"):
            send(t="notify", title=title, body=body)

    def refresh(self):
        try:
            snap = snapshot(self.config["session"])
            current = agents_of(snap)
        except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError):
            self.close_connection()
            if self.up or not self.rendered:
                self.up = False
                self.agents = None
                self.render()
            return
        events = notices(self.agents, current, self.config)
        workspaces = len(snap.get("workspaces", []))
        panes = len(snap.get("panes", []))
        version = snap.get("version") or ""
        changed = (not self.up or self.agents != current or self.workspaces != workspaces
                   or self.panes != panes or self.version != version)
        self.up = True
        self.agents = current
        self.workspaces = workspaces
        self.panes = panes
        self.version = version
        if changed:
            self.render()
        if events:
            self.announce(events)
        self.subscribe(pane_ids(snap))

    def receive_herdr(self):
        try:
            data = self.connection.recv(65536)
        except OSError:
            data = b""
        if not data:
            self.close_connection()
            return
        self.socket_buffer += data
        if len(self.socket_buffer) > 1024 * 1024:
            self.close_connection()
            return
        lines = self.socket_buffer.split(b"\n")
        self.socket_buffer = lines.pop()
        changed = []
        invalidated = False
        for line in lines:
            try:
                message = json.loads(line)
            except ValueError:
                continue
            if "error" in message:
                self.close_connection()
                break
            if message.get("event"):
                invalidated = True
                if message["event"] != "pane.agent_status_changed" or self.agents is None:
                    continue
                data = message.get("data") or {}
                pane = data.get("pane_id")
                state = data.get("agent_status")
                previous = self.agents.get(pane)
                if previous is None or state not in ORDER or previous["state"] == state:
                    continue
                updated = {**previous, "state": state}
                changed.extend(notices({pane: previous}, {pane: updated}, self.config))
                self.agents[pane] = updated
        if invalidated:
            self.next_poll = 0
        if changed:
            self.render()
            self.announce(changed)

    def receive_notchd(self):
        data = os.read(sys.stdin.fileno(), 65536)
        if not data:
            return False
        self.stdin_buffer += data
        if len(self.stdin_buffer) > 1024 * 1024:
            return False
        lines = self.stdin_buffer.split(b"\n")
        self.stdin_buffer = lines.pop()
        for line in lines:
            try:
                message = json.loads(line)
            except ValueError:
                continue
            if message.get("t") == "stop":
                return False
            if message.get("t") in ("init", "config"):
                old_session = self.config["session"]
                incoming = message.get("config") or {}
                self.config = {**DEFAULTS, **incoming}
                if self.config["session"] != old_session:
                    self.close_connection()
                    self.agents = None
                self.next_poll = 0
        return True

    def run(self):
        try:
            while True:
                now = time.monotonic()
                if now >= self.next_poll:
                    self.refresh()
                    try:
                        interval = max(1, min(60, int(self.config["interval"])))
                    except (TypeError, ValueError):
                        interval = 2
                    self.next_poll = time.monotonic() + interval
                timeout = max(0, self.next_poll - time.monotonic())
                for key, _ in self.selector.select(timeout):
                    if key.data == "stdin" and not self.receive_notchd():
                        return
                    if key.data == "herdr" and key.fileobj is self.connection:
                        self.receive_herdr()
        finally:
            self.close_connection()


if __name__ == "__main__":
    Module().run()
