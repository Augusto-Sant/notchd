"""Tests of core/live/codex-live.py that need no GStreamer and no Codex:
what the server's messages mean, the parameters it is sent, the packages
it asks for, the voice's level. The conversation itself is checked by hand
(DEVELOPMENT.md, Codex Live): `--mic` takes a WAV file for that.
"""
import importlib.util
import json
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "..", "core", "live", "codex-live.py")


def load_script():
    spec = importlib.util.spec_from_file_location("codex_live", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


live = load_script()
T = "thread-1"


class Translate(unittest.TestCase):
    def test_transcript_deltas_and_done(self):
        self.assertEqual(live.translate({"method": "thread/realtime/transcript/delta",
                                         "params": {"threadId": T, "role": "user", "delta": " oi"}}, T),
                         [{"transcript": {"role": "user", "delta": " oi"}}])
        self.assertEqual(live.translate({"method": "thread/realtime/transcript/done",
                                         "params": {"threadId": T, "role": "assistant", "text": "Oi!"}}, T),
                         [{"transcript": {"role": "assistant", "text": "Oi!", "done": True}}])

    def test_a_task_is_work(self):
        self.assertEqual(live.translate({"method": "turn/started", "params": {"threadId": T}}, T), [{"working": True}])
        self.assertEqual(live.translate({"method": "turn/completed", "params": {"threadId": T}}, T), [{"working": False}])

    def test_errors_and_closing(self):
        body = json.dumps({"error": {"message": "AVAS requires\n OpenAI-Alpha.", "type": "invalid_request_error"}})
        self.assertEqual(live.translate({"method": "thread/realtime/error", "params": {"threadId": T, "message": body}}, T),
                         [{"error": "AVAS requires OpenAI-Alpha."}])
        self.assertEqual(live.translate({"method": "thread/realtime/closed", "params": {"threadId": T, "reason": "requested"}}, T),
                         [{"status": "closed", "reason": "requested"}])
        self.assertEqual(live.translate({"method": "error", "params": {"error": {"message": "conversation is not running"}}}, T),
                         [{"warning": "conversation is not running"}])

    def test_another_thread_and_the_rest_say_nothing(self):
        self.assertEqual(live.translate({"method": "turn/started", "params": {"threadId": "other"}}, T), [])
        self.assertEqual(live.translate({"method": "mcpServer/startupStatus/updated", "params": {"threadId": T}}, T), [])
        self.assertEqual(live.translate({"method": "account/updated", "params": {}}, T), [])


class Texts(unittest.TestCase):
    def test_error_text(self):
        self.assertEqual(live.error_text("  plain\n  text "), "plain text")
        self.assertEqual(live.error_text(json.dumps({"message": "flat"})), "flat")
        self.assertEqual(live.error_text(""), "The conversation ended with an error.")

    def test_a_codex_without_realtime(self):
        self.assertIn("update Codex", live.rpc_error_text("thread/realtime/start", {"code": -32601, "message": "x"}))
        self.assertIn("update Codex", live.rpc_error_text("thread/realtime/start",
                                                          {"code": -32600, "message": "unknown variant `thread/realtime/start`"}))
        self.assertEqual(live.rpc_error_text("thread/start", {"code": 1, "message": "not logged in"}),
                         "thread/start: not logged in")


class Params(unittest.TestCase):
    def test_thread_never_asks_and_is_read_only_unless_told(self):
        self.assertEqual(live.thread_params("/w", "read-only"),
                         {"cwd": "/w", "ephemeral": True, "sandbox": "read-only", "approvalPolicy": "never"})
        self.assertEqual(live.thread_params("/w", "workspace-write")["sandbox"], "workspace-write")
        self.assertEqual(live.thread_params("/w", "danger-full-access")["sandbox"], "read-only")

    def test_realtime_is_v3_over_webrtc(self):
        p = live.realtime_params(T, "v=0", "")
        self.assertEqual(p, {"threadId": T, "outputModality": "audio", "version": "v3",
                             "transport": {"type": "webrtc", "sdp": "v=0"}})
        self.assertEqual(live.realtime_params(T, "v=0", "maple")["voice"], "maple")

    def test_own_dir(self):
        self.assertEqual(live.own_dir("/home/u/Documentos\n", "/home/u"), "/home/u/Documentos/notchd")
        self.assertEqual(live.own_dir("", "/home/u"), "/home/u/Documents/notchd")


class Packages(unittest.TestCase):
    def test_each_package_once_in_order(self):
        have = {"webrtcbin", "opusenc", "opusdec", "audioconvert", "audioresample", "audiotestsrc", "volume", "appsink", "appsrc"}
        self.assertEqual(live.missing_packages(lambda n: n in have),
                         ["gst-plugins-bad", "libnice", "gst-plugins-good"])
        self.assertEqual(live.missing_packages(lambda n: True), [])


class Level(unittest.TestCase):
    def test_rms_db(self):
        self.assertEqual(live.rms_db([]), -100.0)
        self.assertEqual(live.rms_db([0] * 960), -100.0)
        self.assertEqual(live.rms_db([32767, -32768] * 480), 0.0)
        self.assertAlmostEqual(live.rms_db([3277, -3277] * 480), -20.0, places=0)


class Commands(unittest.TestCase):
    def test_parse_command(self):
        self.assertEqual(live.parse_command('{"mute": true}'), ("mute", True))
        self.assertEqual(live.parse_command('{"stop": true}'), ("stop", None))
        self.assertIsNone(live.parse_command('{"mute": "yes"}'))
        self.assertIsNone(live.parse_command("nonsense"))
        self.assertIsNone(live.parse_command("[1]"))


def gstreamer():
    """Gst, if GStreamer and the echo canceller are there; else None."""
    try:
        import gi
        gi.require_version("Gst", "1.0")
        from gi.repository import Gst
    except (ImportError, ValueError):
        return None
    Gst.init(None)
    if not all(Gst.ElementFactory.find(e) for e in live.ECHO_ELEMENTS + ["appsrc", "audiotestsrc"]):
        return None
    return Gst


class EchoBranch(unittest.TestCase):
    """Bug: the echo's appsrc had caps without a layout, not fixed, and the
    first voice pushed into it ended the session ("Internal data stream
    error"). The branch as the bridge builds it takes the voice."""
    tier = "integration"  # a real GStreamer pipeline

    def test_the_voice_goes_into_the_echo_canceller(self):
        Gst = gstreamer()
        if Gst is None:
            self.skipTest("no GStreamer with webrtcdsp")
        pipe = Gst.parse_launch(
            "audiotestsrc is-live=true wave=silence ! %s ! webrtcdsp probe=probe delay-agnostic=true ! fakesink %s"
            % (live.VOICE_CAPS, live.ECHO_BRANCH))
        pipe.set_state(Gst.State.PLAYING)
        try:
            src = pipe.get_by_name("echo")
            bus = pipe.get_bus()
            for _ in range(25):
                src.emit("push-buffer", Gst.Buffer.new_wrapped(bytes(1920)))
                message = bus.timed_pop_filtered(40 * Gst.MSECOND, Gst.MessageType.ERROR)
                self.assertIsNone(message, message and message.parse_error()[0].message)
        finally:
            pipe.set_state(Gst.State.NULL)


if __name__ == "__main__":
    unittest.main()
