"""Regressions for the optional Herdr user module's completion notices."""

import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch


SOURCE = Path(__file__).resolve().parents[1] / "docs/examples/modules/herdr/run.py"
spec = importlib.util.spec_from_file_location("notchd_herdr_example", SOURCE)
herdr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(herdr)


def agent(state):
    return {"pane": "w1:p1", "agent": "Codex", "state": state,
            "title": "Task", "where": "work", "seq": 1}


class CompletionNotices(unittest.TestCase):
    def test_working_to_idle_is_a_completion(self):
        before = {"w1:p1": agent("working")}
        after = {"w1:p1": agent("idle")}
        self.assertEqual(herdr.notices(before, after, herdr.DEFAULTS), [after["w1:p1"]])
        self.assertEqual(herdr.notices(before, after, {**herdr.DEFAULTS, "done": False}), [])
        self.assertEqual(herdr.notices(None, after, herdr.DEFAULTS), [])
        self.assertEqual(herdr.notices({"w1:p1": agent("blocked")}, after,
                                       herdr.DEFAULTS), [])

    def test_status_event_announces_before_next_snapshot(self):
        class Socket:
            def recv(self, _size):
                return (json.dumps({"event": "pane.agent_status_changed",
                                    "data": {"pane_id": "w1:p1", "agent_status": "idle"}})
                        + "\n").encode()

        module = herdr.Module.__new__(herdr.Module)
        module.connection = Socket()
        module.socket_buffer = b""
        module.agents = {"w1:p1": agent("working")}
        module.config = herdr.DEFAULTS.copy()
        module.next_poll = 42
        rendered = []
        announced = []
        module.render = lambda: rendered.append(True)
        module.announce = lambda events: announced.extend(events)

        module.receive_herdr()

        self.assertEqual(module.agents["w1:p1"]["state"], "idle")
        self.assertEqual([a["state"] for a in announced], ["idle"])
        self.assertEqual(rendered, [True])
        self.assertEqual(module.next_poll, 0)
        self.assertEqual(herdr.notices(module.agents, module.agents, module.config), [])

    def test_snapshot_completion_updates_compact_and_requests_peek(self):
        module = herdr.Module.__new__(herdr.Module)
        module.agents = {"w1:p1": agent("working")}
        module.config = herdr.DEFAULTS.copy()
        module.workspaces = 1
        module.panes = 1
        module.version = "0.9.1"
        module.up = True
        module.rendered = True
        module.subscribe = lambda _panes: None
        snap = {"agents": [{"pane_id": "w1:p1", "agent": "codex",
                            "agent_status": "idle", "state_change_seq": 2}],
                "panes": [{"pane_id": "w1:p1"}],
                "workspaces": [{"workspace_id": "w1", "label": "work"}],
                "version": "0.9.1"}
        messages = []

        with patch.object(herdr, "snapshot", return_value=snap), \
                patch.object(herdr, "send", side_effect=lambda **msg: messages.append(msg)):
            module.refresh()

        self.assertEqual(module.agents["w1:p1"]["state"], "idle")
        self.assertTrue(any(msg.get("t") == "peek" for msg in messages))
        compact = next(msg["tree"] for msg in messages
                       if msg.get("t") == "view" and msg.get("slot") == "compact")
        self.assertIn("0 working", [child.get("text") for child in compact["children"]])
