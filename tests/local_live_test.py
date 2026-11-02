"""Tests of core/live/local-live.py, the cascade bridge (Voxtype, an agent's
text session, a LocalAI voice): the rules (voice activity, sentences,
markdown, WAVs, the agent's command line and stream) and the whole bridge run
with a fake LocalAI (an HTTP server on 127.0.0.1, in this process) and fake
voxtype, pacat, codex and claude on PATH and a WAV for the microphone.
Nothing here touches your config, your sessions, a real LocalAI or the sound.
"""
import importlib.util
import json
import math
import os
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "..", "core", "live", "local-live.py")


def load_script():
    spec = importlib.util.spec_from_file_location("local_live", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


live = load_script()


def pcm(db, seconds=live.FRAME_S, rate=16000):
    amp = 32767 * 10 ** (db / 20) * math.sqrt(2)
    n = int(rate * seconds)
    return b"".join(struct.pack("<h", int(amp * math.sin(2 * math.pi * 440 * i / rate))) for i in range(n))


class Level(unittest.TestCase):
    def test_silence_and_a_tone(self):
        self.assertEqual(live.rms_db(b"\0" * 3200), -100.0)
        self.assertEqual(live.rms_db(b""), -100.0)
        self.assertAlmostEqual(live.rms_db(pcm(-20)), -20, delta=0.5)


class VadRules(unittest.TestCase):
    def run_vad(self, seconds_db, **kw):
        vad = live.Vad(**kw)
        events = []
        for secs, db in seconds_db:
            for _ in range(round(secs / live.FRAME_S)):
                e = vad.feed(db)
                if e:
                    events.append(e)
        return events, vad

    def test_calibrates_then_hears_an_utterance(self):
        events, vad = self.run_vad([(0.6, -70), (1.0, -20), (1.0, -70)])
        self.assertEqual(events, ["start", "end"])
        self.assertTrue(vad.ready)
        self.assertGreaterEqual(vad.threshold, -58)

    def test_a_click_is_dropped(self):
        events, _ = self.run_vad([(0.6, -70), (0.1, -20), (1.0, -70)])
        self.assertEqual(events, ["start", "drop"])

    def test_a_short_pause_does_not_split_it(self):
        events, _ = self.run_vad([(0.6, -70), (0.5, -20), (0.4, -70), (0.5, -20), (1.0, -70)])
        self.assertEqual(events, ["start", "end"])

    def test_nothing_during_calibration(self):
        events, vad = self.run_vad([(0.3, -20)])
        self.assertEqual(events, [])
        self.assertFalse(vad.ready)

    def test_noise_raises_the_threshold(self):
        _, quiet = self.run_vad([(0.6, -80)])
        _, noisy = self.run_vad([(0.6, -35)])
        self.assertGreater(noisy.threshold, quiet.threshold)
        events, _ = self.run_vad([(0.6, -35), (1.0, -33), (1.0, -35)])
        self.assertEqual(events, [])

    def test_noise_that_starts_while_you_talk_still_ends_it(self):
        # Music starts as you speak and goes on: the floor follows it.
        events, _ = self.run_vad([(0.6, -85), (1.0, -20), (10.0, -45)])
        self.assertEqual(events, ["start", "end"])

    def test_a_breath_between_words_keeps_it_going(self):
        # Under the start threshold but over the end one: still speaking.
        events, _ = self.run_vad([(0.6, -70), (0.5, -20), (1.0, -55), (1.0, -70)])
        self.assertEqual(events, ["start", "end"])
        events, vad = self.run_vad([(0.6, -70), (0.5, -20), (1.0, -55)])
        self.assertTrue(vad.speaking)

    def noise(self, secs, floor):
        # A room's noise: around `floor`, a dB up and down.
        frames = round(secs / live.FRAME_S)
        return [(live.FRAME_S, floor + (1 if i % 2 else -1)) for i in range(frames)]

    def test_speech_is_heard_by_how_far_over_the_room_it_is(self):
        # Whatever the room and the mixer's volume (a loud room, a quiet one,
        # a mic turned down), speech 15 dB over the noise is an utterance,
        # 5 dB over it is nothing.
        for floor in range(-80, -35, 5):
            with self.subTest(floor=floor):
                room = self.noise(1.0, floor)
                events, _ = self.run_vad(room + [(1.0, floor + 15)] + room)
                self.assertEqual(events, ["start", "end"])
                events, _ = self.run_vad(room + [(1.0, floor + 5)] + room)
                self.assertEqual(events, [])

    def test_digital_silence_is_not_the_floor(self):
        # A source that sends zeros when quiet (noise suppression, a mic
        # waking up): those frames are left out, and speech after them, loud
        # or quiet, is an utterance from its first word.
        for voice in range(-65, -10, 10):
            with self.subTest(voice=voice):
                events, _ = self.run_vad([(1.0, -100), (1.0, voice), (1.0, -100)])
                self.assertEqual(events, ["start", "end"])

    def test_a_long_speech_is_cut(self):
        events, _ = self.run_vad([(0.6, -70), (3.0, -20)], max_utt=2.0)
        self.assertEqual(events[:2], ["start", "end"])


class Sentences(unittest.TestCase):
    def test_streamed_text_splits_when_a_sentence_ends(self):
        out, rest = live.split_sentences("Primeiro eu li o arquivo. Depois rodei os te")
        self.assertEqual(out, ["Primeiro eu li o arquivo."])
        self.assertEqual(rest, "Depois rodei os te")
        out, rest = live.split_sentences(rest + "stes. Tudo passou!", final=True)
        self.assertEqual(out, ["Depois rodei os testes. Tudo passou!"])
        self.assertEqual(rest, "")

    def test_a_stop_at_the_end_waits(self):
        self.assertEqual(live.split_sentences("Eu li o arquivo inteiro."), ([], "Eu li o arquivo inteiro."))
        self.assertEqual(live.split_sentences("Eu li o arquivo inteiro.", final=True), (["Eu li o arquivo inteiro."], ""))

    def test_short_ones_join_the_next(self):
        out, rest = live.split_sentences("Ok. Rodei os testes e todos passaram. ")
        self.assertEqual(out, ["Ok. Rodei os testes e todos passaram."])

    def test_decimals_and_line_breaks(self):
        out, rest = live.split_sentences("A versão 3.5 do pacote foi instalada\nfeito ")
        self.assertEqual(out, ["A versão 3.5 do pacote foi instalada"])


class Markdown(unittest.TestCase):
    def test_what_is_not_spoken(self):
        self.assertEqual(live.strip_markdown("**Feito**: rodei `make check`."), "Feito: rodei make check.")
        self.assertEqual(live.strip_markdown("Veja [a doc](https://x.dev/a) e https://y.dev agora"), "Veja a doc e agora")
        self.assertEqual(live.strip_markdown("# Título\n- um\n- dois\n1. três"), "Título\num\ndois\ntrês")
        self.assertEqual(live.strip_markdown("Antes\n```py\nprint(1)\n```\nDepois"), "Antes\nDepois")
        self.assertEqual(live.strip_markdown("snake_case fica"), "snake_case fica")

    def test_transcript_output(self):
        self.assertEqual(live.clean_transcript(" [BLANK_AUDIO]\n"), "")
        self.assertEqual(live.clean_transcript(" Olá, tudo bem?\n (música) \n"), "Olá, tudo bem?")
        noisy = "whisper_full_with_state: auto-detected language: pt (p = 0.99)\nggml_cuda_init: found 1 CUDA devices\n" \
                "2026-11-01T21:00:00Z INFO loaded\nRode os testes,\npor favor.\n"
        self.assertEqual(live.clean_transcript(noisy), "Rode os testes, por favor.")
        # what `voxtype -q transcribe` really prints on stdout (the model's own logs go to stderr)
        real = 'Loading audio file: "utterance.wav"\nAudio format: 16000 Hz, 1 channel(s), Int\n' \
               'Processing 63387 samples (3.96s)...\n\nClaro, eu já terminei a tarefa.\n'
        self.assertEqual(live.clean_transcript(real), "Claro, eu já terminei a tarefa.")


class AgentCommands(unittest.TestCase):
    def test_codex(self):
        self.assertEqual(live.agent_argv("codex", "", "last", "/w", "read-only"),
                         ["codex", "exec", "--json", "--skip-git-repo-check", "-s", "read-only", "-C", "/w",
                          "resume", "--last", "-"])
        self.assertEqual(live.agent_argv("codex", "codex -p work", "abc", "/w", "workspace-write")[:3],
                         ["codex", "-p", "work"])
        self.assertEqual(live.agent_argv("codex", "", "abc", "/w", "workspace-write")[-3:], ["resume", "abc", "-"])
        new = live.agent_argv("codex", "", "new", "/w", "bogus")
        self.assertNotIn("resume", new)
        self.assertIn("read-only", new)
        self.assertEqual(new[-1], "-")

    def test_claude(self):
        base = ["claude", "-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages",
                "--permission-mode"]
        self.assertEqual(live.agent_argv("claude", "", "last", "/w", "read-only"), base + ["plan", "--continue"])
        self.assertEqual(live.agent_argv("claude", "", "s1", "/w", "workspace-write"),
                         base + ["acceptEdits", "--resume", "s1"])
        self.assertEqual(live.agent_argv("claude", "", "new", "/w", "read-only"), base + ["plan"])


class AgentStreams(unittest.TestCase):
    def parse(self, agent, *objs):
        out = []
        for o in objs:
            out += live.parse_agent_line(agent, json.dumps(o))
        return out

    def test_codex(self):
        self.assertEqual(self.parse("codex",
                                    {"type": "thread.started", "thread_id": "T1"},
                                    {"type": "turn.started"},
                                    {"type": "item.completed", "item": {"type": "command_execution", "command": "ls"}},
                                    {"type": "item.completed", "item": {"type": "agent_message", "text": "Oi"}},
                                    {"type": "turn.completed", "usage": {}}),
                         [("session", "T1"), ("message", "Oi"), ("done",)])
        self.assertEqual(self.parse("codex", {"type": "turn.failed", "error": {"message": "nope"}}), [("error", "nope")])
        self.assertEqual(live.parse_agent_line("codex", "Reading additional input from stdin..."), [])

    def test_claude(self):
        delta = {"type": "stream_event", "session_id": "S1",
                 "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "oi"}}}
        self.assertEqual(self.parse("claude",
                                    {"type": "system", "subtype": "init", "session_id": "S1"},
                                    delta,
                                    {"type": "stream_event", "event": {"type": "content_block_stop"}},
                                    {"type": "assistant", "message": {"content": [{"type": "text", "text": "oi"}]}},
                                    {"type": "result", "is_error": False, "session_id": "S1", "result": "oi"}),
                         [("session", "S1"), ("delta", "oi"), ("break",), ("session", "S1"), ("done",)])
        self.assertEqual(self.parse("claude", {"type": "result", "is_error": True, "result": "bad"}),
                         [("error", "bad"), ("done",)])


class Normalize(unittest.TestCase):
    def test_quiet_speech_is_raised_loud_left(self):
        import array as arr
        quiet = arr.array("h", [100, -200, 50]).tobytes()
        out = arr.array("h"); out.frombytes(live.normalize(quiet))
        self.assertEqual(max(abs(x) for x in out), 200 * 30)
        loud = arr.array("h", [30000, -1000]).tobytes()
        self.assertEqual(live.normalize(loud), loud)
        self.assertEqual(live.normalize(b"\0\0" * 4), b"\0\0" * 4)


class Misc(unittest.TestCase):
    def test_commands_and_paths(self):
        self.assertEqual(live.parse_command('{"mute": true}'), ("mute", True))
        self.assertEqual(live.parse_command('{"stop": true}'), ("stop", None))
        self.assertIsNone(live.parse_command("nope"))
        self.assertEqual(live.work_dir("", "/h"), "/h/Documents/notchd")
        self.assertEqual(live.work_dir("~/code", "/h"), "/h/code")



def wav_bytes(samples, rate, tag=1, bits=16, channels=1, extra=b""):
    if tag == 3:
        body = struct.pack("<%df" % len(samples), *samples)
    else:
        body = struct.pack("<%dh" % len(samples), *samples)
    fmt = struct.pack("<HHIIHH", tag, channels, rate, rate * channels * bits // 8, channels * bits // 8, bits)
    return (b"RIFF" + struct.pack("<I", 36 + len(body)) + b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + extra
            + b"data" + struct.pack("<I", len(body)) + body)


class Wavs(unittest.TestCase):
    """What LocalAI's voices return: Piper writes 16-bit PCM, Pocket TTS 32-bit float."""

    def test_16_bit(self):
        out, rate = live.wav_to_pcm16(wav_bytes([0, 1000, -1000, 32767], 22050))
        self.assertEqual((struct.unpack("<4h", out), rate), ((0, 1000, -1000, 32767), 22050))

    def test_float(self):
        out, rate = live.wav_to_pcm16(wav_bytes([0.0, 0.5, -0.5, 2.0, -2.0], 24000, tag=3, bits=32))
        self.assertEqual(rate, 24000)
        self.assertEqual(struct.unpack("<5h", out), (0, 16384, -16384, 32767, -32767))

    def test_stereo_is_averaged(self):
        out, rate = live.wav_to_pcm16(wav_bytes([100, 300, -50, 50], 16000, channels=2))
        self.assertEqual(struct.unpack("<2h", out), (200, 0))

    def test_a_data_size_larger_than_the_file_reads_to_its_end(self):
        data = bytearray(wav_bytes([1, 2, 3, 4], 16000))
        data[-12:-8] = struct.pack("<I", 0xFFFFFFFF)  # a streaming writer's size
        self.assertEqual(struct.unpack("<4h", live.wav_to_pcm16(bytes(data))[0]), (1, 2, 3, 4))

    def test_other_chunks_are_skipped_and_garbage_is_none(self):
        out, rate = live.wav_to_pcm16(wav_bytes([5, 6], 16000, extra=b"LIST" + struct.pack("<I", 3) + b"abc\0"))
        self.assertEqual(struct.unpack("<2h", out), (5, 6))
        self.assertIsNone(live.wav_to_pcm16(b""))
        self.assertIsNone(live.wav_to_pcm16(b'{"error": {"code": 500}}'))
        self.assertIsNone(live.wav_to_pcm16(wav_bytes([1], 16000, tag=1, bits=16)[:30]))
        self.assertIsNone(live.wav_to_pcm16(wav_bytes([1, 2], 16000, tag=1, bits=16).replace(b"\x10\x00\x00\x00\x01\x00", b"\x10\x00\x00\x00\x07\x00", 1)))


class Voices(unittest.TestCase):
    """The voices are the registry's: an engine and a language each."""
    voices = live.known_voices()
    languages = sorted({v["language"] for v in voices})

    def model(self, language, engine):
        return next(v["model"] for v in self.voices if v["language"] == language and v["engine"] == engine)

    def component(self, language, engine):
        return next(v["component"] for v in self.voices if v["language"] == language and v["engine"] == engine)

    def test_every_language_has_both_engines(self):
        self.assertGreaterEqual(len(self.languages), 7)
        for code in self.languages:
            self.assertEqual([v["engine"] for v in self.voices if v["language"] == code], ["piper", "pocket"], code)
        self.assertEqual(len({v["model"] for v in self.voices}), len(self.voices))

    def test_the_first_installed_voice_of_the_language_piper_before_pocket(self):
        for code in self.languages:
            piper, pocket = self.model(code, "piper"), self.model(code, "pocket")
            self.assertEqual(live.pick_voice_model(["x", pocket, piper], language=code), (piper, None), code)
            self.assertEqual(live.pick_voice_model(["x", pocket], language=code), (pocket, None), code)
            # the other languages' voices are not this one's
            others = [v["model"] for v in self.voices if v["language"] != code]
            self.assertEqual(live.pick_voice_model(others, language=code), (None, self.component(code, "piper")), code)

    def test_an_engine_is_chosen_within_the_language(self):
        for code in self.languages:
            piper, pocket = self.model(code, "piper"), self.model(code, "pocket")
            both = [piper, pocket]
            self.assertEqual(live.pick_voice_model(both, engine="pocket", language=code), (pocket, None), code)
            self.assertEqual(live.pick_voice_model(both, engine="piper", language=code), (piper, None), code)
            # the engine that isn't there names its own component, not the other engine's
            self.assertEqual(live.pick_voice_model([piper], engine="pocket", language=code), (None, self.component(code, "pocket")), code)
            self.assertEqual(live.pick_voice_model([pocket], engine="piper", language=code), (None, self.component(code, "piper")), code)

    def test_none_installed_names_the_components_of_the_language(self):
        for code in self.languages:
            self.assertEqual(live.pick_voice_model(["gpt"], language=code), (None, self.component(code, "piper")), code)
            self.assertEqual(live.pick_voice_model([], language=code), (None, self.component(code, "piper")), code)

    def test_the_default_language_is_portuguese_and_a_language_without_voices_falls_back_to_it(self):
        self.assertEqual(live.DEFAULT_LANGUAGE, "pt")
        self.assertEqual(live.pick_voice_model([]), (None, "voice-piper-pt-br"))
        self.assertEqual(live.pick_voice_model(["x", "pocket-pt"]), ("pocket-pt", None))
        self.assertEqual(live.pick_voice_model([], language="xx"), (None, "voice-piper-pt-br"))
        self.assertEqual(live.pick_voice_model(["voice-pt_BR-faber-medium"], language="xx"), ("voice-pt_BR-faber-medium", None))

    def test_a_chosen_model(self):
        self.assertEqual(live.pick_voice_model(["a", "b"], "b"), ("b", None))
        self.assertEqual(live.pick_voice_model(["a"], "pocket-pt"), (None, "voice-pocket-pt"))
        self.assertEqual(live.pick_voice_model(["a"], "somebody-elses"), (None, None))
        for v in self.voices:  # a model by name is that model, whatever the language says
            self.assertEqual(live.pick_voice_model([v["model"]], v["model"], language="nl"), (v["model"], None))
            self.assertEqual(live.pick_voice_model([], v["model"], language="nl"), (None, v["component"]))

    def test_the_registry_the_bridge_reads_can_be_replaced_and_a_bad_one_is_empty(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "c.json")
            with open(path, "w") as f:
                json.dump({"components": [{"id": "v1", "kind": "localai", "engine": "piper", "language": "xx",
                                           "model": {"name": "m1"}, "warmup": "Hi."}, {"id": "no-engine", "kind": "localai"}]}, f)
            old = os.environ.get("NOTCHD_COMPONENTS_FILE")
            try:
                os.environ["NOTCHD_COMPONENTS_FILE"] = path
                self.assertEqual(live.known_voices(), [{"model": "m1", "component": "v1", "engine": "piper", "language": "xx", "warmup": "Hi."}])
                with open(path, "w") as f:
                    f.write("not json")
                self.assertEqual(live.known_voices(), [])
            finally:
                if old is None:
                    del os.environ["NOTCHD_COMPONENTS_FILE"]
                else:
                    os.environ["NOTCHD_COMPONENTS_FILE"] = old

    def test_the_request(self):
        self.assertEqual(live.speech_request("m", "oi"), {"model": "m", "input": "oi"})
        self.assertEqual(live.speech_request("m", "oi", "rafael"), {"model": "m", "input": "oi", "voice": "rafael"})


FAKES = {
    "voxtype": """#!/usr/bin/env python3
import os, sys
open(os.path.join(os.environ["FAKE_DIR"], "voxtype-args"), "a").write(" ".join(sys.argv[1:]) + "\\n")
print("whisper_full_with_state: auto-detected language: pt (p = 0.99)", file=sys.stderr)
print('Loading audio file: "x.wav"')
print("Audio format: 16000 Hz, 1 channel(s), Int")
print("Processing 12000 samples (0.75s)...")
print()
print(" Rode os testes, por favor.")
""",
    "pacat": """#!/usr/bin/env python3
import os, sys
open(os.path.join(os.environ["FAKE_DIR"], "pacat-args"), "a").write(" ".join(sys.argv[1:]) + "\\n")
data = sys.stdin.buffer.read()
open(os.path.join(os.environ["FAKE_DIR"], "pacat-bytes"), "a").write("%d\\n" % len(data))
""",
    "agent": """#!/usr/bin/env python3
import json, os, sys, time
d = os.environ["FAKE_DIR"]
prompt = sys.stdin.read()
open(os.path.join(d, "agent-calls"), "a").write(json.dumps({"argv": sys.argv[1:], "stdin": prompt, "cwd": os.getcwd()}) + "\\n")
mode = os.environ.get("FAKE_AGENT", "")
answer = os.environ.get("FAKE_ANSWER", "**Rodei** os testes. Todos passaram.")
if mode == "hang":
    open(os.path.join(d, "agent-started"), "w").write("1")
    time.sleep(60)
name = os.path.basename(sys.argv[0])
if name == "claude":
    def ev(o): print(json.dumps(o), flush=True)
    ev({"type": "system", "subtype": "init", "session_id": "S-claude"})
    for t in ["Rodei os testes. ", "Todos passaram."]:
        ev({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": t}}})
    ev({"type": "stream_event", "event": {"type": "content_block_stop"}})
    ev({"type": "result", "is_error": False, "session_id": "S-claude", "result": "x"})
else:
    print("Reading additional input from stdin...", file=sys.stderr)
    print(json.dumps({"type": "thread.started", "thread_id": "T-codex"}), flush=True)
    print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": answer}}), flush=True)
    print(json.dumps({"type": "turn.completed"}), flush=True)
""",
}


FAKE_LOCALAI_BINARY = """#!/usr/bin/env python3
# A LocalAI program: records how it was started, answers like the real one.
import json, os, struct, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
d = os.environ["FAKE_DIR"]
open(os.path.join(d, "localai-argv"), "w").write(" ".join(sys.argv[1:]))
open(os.path.join(d, "localai-pid"), "w").write(str(os.getpid()))
open(os.path.join(d, "localai-home"), "w").write(os.environ.get("HF_HOME", ""))
host, port = sys.argv[sys.argv.index("--address") + 1].rsplit(":", 1)

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, body, kind="application/json"):
        self.send_response(200); self.send_header("Content-Length", str(len(body))); self.send_header("Content-Type", kind)
        self.end_headers(); self.wfile.write(body)
    def do_GET(self):
        self.send(b"OK" if self.path == "/readyz" else json.dumps({"data": [{"id": "voice-pt_BR-faber-medium"}]}).encode())
    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length") or 0))
        pcm = struct.pack("<h", 1000) * 6000
        wav = b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 1, 16000, 32000, 2, 16) + b"data" + struct.pack("<I", len(pcm)) + pcm
        self.send(wav, "audio/x-wav")

HTTPServer((host, int(port)), H).serve_forever()
"""


def localai_ready(url):
    import urllib.request
    try:
        return urllib.request.urlopen(url + "/readyz", timeout=3).status == 200
    except OSError:
        return False


class FakeLocalAI(BaseHTTPRequestHandler):
    """/readyz, /v1/models and /v1/audio/speech (a 16-bit 16 kHz WAV, or a
    24 kHz float one for the model "pocket-pt"; 500 for a text with FALHA)."""
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def reply(self, code, body, kind="application/json"):
        self.send_response(code)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/readyz":
            return self.reply(200, b"OK")
        if self.path == "/v1/models":
            data = [{"id": m, "object": "model"} for m in self.server.models]
            return self.reply(200, json.dumps({"object": "list", "data": data}).encode())
        self.reply(404, b"{}")

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)) or b"{}")
        if self.path != "/v1/audio/speech":
            return self.reply(404, b"{}")
        self.server.requests.append(body)
        if body["model"] not in self.server.models:
            return self.reply(500, json.dumps({"error": {"code": 500, "message": "model not found: " + body["model"]}}).encode())
        if "FALHA" in body["input"] or (self.server.fail_warmup and body["input"] == "Olá."):
            return self.reply(500, json.dumps({"error": {"code": 500, "message": "backend crashed"}}).encode())
        n = 6000
        if body["model"] == "pocket-pt":
            return self.reply(200, wav_bytes([0.1] * n, 24000, tag=3, bits=32), "audio/x-wav")
        self.reply(200, wav_bytes([1000] * n, 16000), "audio/x-wav")


class Bridge(unittest.TestCase):
    """The whole bridge, with fakes. Every run is bounded by `timeout -k`."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="notchd-ll-test-")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.bin = os.path.join(self.dir, "bin")
        os.makedirs(self.bin)
        for name, body in FAKES.items():
            names = ["codex", "claude"] if name == "agent" else [name]
            for n in names:
                path = os.path.join(self.bin, n)
                with open(path, "w") as f:
                    f.write(body)
                os.chmod(path, 0o755)
        self.ai = ThreadingHTTPServer(("127.0.0.1", 0), FakeLocalAI)
        self.ai.models = ["voice-pt_BR-faber-medium", "pocket-pt"]
        self.ai.requests = []
        self.ai.fail_warmup = False
        self.ai.daemon_threads = True
        threading.Thread(target=self.ai.serve_forever, daemon=True).start()
        self.addCleanup(self.ai.server_close)
        self.addCleanup(self.ai.shutdown)
        self.url = "http://127.0.0.1:%d" % self.ai.server_address[1]
        self.wav = os.path.join(self.dir, "mic.wav")
        with wave.open(self.wav, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(16000)
            w.writeframes(pcm(-80, 0.8) + pcm(-15, 1.2) + pcm(-80, 0.4))
        self.work = os.path.join(self.dir, "work")
        self.procs = []
        self.addCleanup(self.reap)

    def reap(self):
        for p in self.procs:
            if p.poll() is None:
                p.kill()
                p.wait()

    def env(self, **extra):
        env = {"PATH": self.bin + ":/usr/bin:/bin", "HOME": self.dir, "FAKE_DIR": self.dir,
               "XDG_STATE_HOME": os.path.join(self.dir, "state"), "XDG_DATA_HOME": os.path.join(self.dir, "data"),
               "NOTCHD_LIVE_MIC_SPEED": "10", "TMPDIR": self.dir}
        env.update(extra)
        return env

    def start(self, *args, **env):
        cmd = ["timeout", "-k", "2", "40", sys.executable, SCRIPT, "--mic", self.wav, "--cwd", self.work,
               "--localai-url", self.url] + list(args)
        p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                             env=self.env(**env), text=True)
        self.procs.append(p)
        self.events = []
        self.cond = threading.Condition()

        def read():
            for line in p.stdout:
                with self.cond:
                    self.events.append(json.loads(line))
                    self.cond.notify_all()
            with self.cond:
                self.cond.notify_all()
        threading.Thread(target=read, daemon=True).start()
        return p

    def wait_for(self, pred, seconds=25):
        end = time.time() + seconds
        with self.cond:
            while not any(pred(e) for e in self.events):
                left = end - time.time()
                if left <= 0:
                    self.fail("never saw it; events: %r" % self.events[-12:])
                self.cond.wait(left)

    def one_turn(self, *args, **env):
        p = self.start(*args, **env)
        self.wait_for(lambda e: e.get("transcript", {}).get("role") == "assistant" and e["transcript"].get("done"))
        self.wait_for(lambda e: e.get("working") is False)
        p.stdin.write('{"stop": true}\n')
        p.stdin.flush()
        self.assertEqual(p.wait(timeout=15), 0)
        return list(self.events)

    def file(self, name):
        with open(os.path.join(self.dir, name)) as f:
            return f.read()

    def lines(self, name):
        return [json.loads(l) for l in self.file(name).splitlines()]

    def leftovers(self):
        out = subprocess.run(["ps", "-eo", "pid,args"], capture_output=True, text=True).stdout
        return [l for l in out.splitlines() if self.dir in l and "ps -eo" not in l]

    def test_codex_turn_end_to_end(self):
        events = self.one_turn("--agent", "codex", "--session", "new")
        statuses = [e["status"] for e in events if "status" in e]
        self.assertEqual(statuses[0], "starting")
        self.assertIn("live", statuses)
        self.assertEqual(statuses[-1], "closed")
        user = [e["transcript"] for e in events if e.get("transcript", {}).get("role") == "user"]
        self.assertEqual(user, [{"role": "user", "text": "Rode os testes, por favor.", "done": True}])
        done = [e["transcript"] for e in events if e.get("transcript", {}).get("role") == "assistant" and e["transcript"].get("done")]
        self.assertEqual(done[0]["text"], "**Rodei** os testes. Todos passaram.")
        self.assertTrue(any(e.get("working") is True for e in events))
        self.assertTrue(any("mic" in e for e in events))
        levels = [e["level"] for e in events if "level" in e]
        self.assertTrue(any(l > -60 for l in levels), levels)
        self.assertEqual(levels[-1], -100)
        # the order: the question before the answer, working around both
        idx = lambda pred: next(i for i, e in enumerate(events) if pred(e))
        self.assertLess(idx(lambda e: e.get("working") is True),
                        idx(lambda e: e.get("transcript", {}).get("role") == "assistant" and e["transcript"].get("done")))
        # what each part was given
        self.assertEqual(self.file("voxtype-args").split()[:2], ["-q", "transcribe"])  # Voxtype's own model and language
        call = self.lines("agent-calls")[0]
        self.assertEqual(call["argv"][:2], ["exec", "--json"])
        self.assertNotIn("resume", call["argv"])
        self.assertIn("Rode os testes, por favor.", call["stdin"])
        self.assertIn(live.SPOKEN_HINT, call["stdin"])
        self.assertEqual(os.path.realpath(call["cwd"]), os.path.realpath(self.work))
        # the answer was spoken, markdown stripped, sentence by sentence
        self.assertEqual([r["input"] for r in self.ai.requests], [live.WARMUP_TEXT, "Rodei os testes. Todos passaram."])
        self.assertEqual({r["model"] for r in self.ai.requests}, {"voice-pt_BR-faber-medium"})
        self.assertTrue(all("voice" not in r for r in self.ai.requests))  # the model's own voice
        self.assertIn("--rate 16000 --channels 1", self.file("pacat-args"))
        self.assertEqual(int(self.file("pacat-bytes").split()[0]), 6000 * 2)
        time.sleep(0.3)
        self.assertEqual(self.leftovers(), [])

    def test_claude_resumes_the_last_session(self):
        events = self.one_turn("--agent", "claude", "--session", "last")
        call = self.lines("agent-calls")[0]
        self.assertIn("--continue", call["argv"])
        self.assertIn("stream-json", call["argv"])
        deltas = "".join(e["transcript"].get("delta", "") for e in events if e.get("transcript", {}).get("role") == "assistant")
        self.assertEqual(deltas.strip(), "Rodei os testes. Todos passaram.")
        self.assertTrue(any(e.get("transcript", {}).get("done") and e["transcript"]["role"] == "assistant" for e in events))

    def test_an_explicit_session_is_resumed(self):
        self.one_turn("--agent", "codex", "--session", "abc-123")
        argv = self.lines("agent-calls")[0]["argv"]
        self.assertEqual(argv[-3:], ["resume", "abc-123", "-"])

    def test_missing_binaries_are_asked_for(self):
        p = subprocess.run([sys.executable, SCRIPT, "--mic", self.wav, "--localai-url", self.url],
                           capture_output=True, text=True, timeout=20,
                           env={"PATH": "/nonexistent", "HOME": self.dir, "XDG_STATE_HOME": self.dir})
        self.assertEqual(p.returncode, 2)
        err = [json.loads(l) for l in p.stdout.splitlines() if "error" in l][0]
        self.assertEqual(sorted(err["packages"]), ["libpulse", "voxtype-bin"])
        self.assertEqual(sorted(err["components"]), ["libpulse", "voxtype"])
        self.assertNotIn("make ", err["error"])
        self.assertEqual(self.ai.requests, [])

    def error_of(self, *args, **env):
        """The conversation that ends by itself with an error: (exit code, the error event)."""
        p = subprocess.Popen(["timeout", "-k", "2", "30", sys.executable, SCRIPT, "--mic", self.wav, "--cwd", self.work] + list(args),
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
                             env=self.env(**env))
        self.procs.append(p)
        out = p.stdout.read()  # until it ends; stdin stays open, as the shell's does
        code = p.wait(timeout=20)
        p.stdout.close()
        p.stdin.close()
        errors = [e for e in map(json.loads, out.splitlines()) if "error" in e]
        self.assertEqual(len(errors), 1, out)
        return code, errors[0]

    def test_without_localai_it_says_which_component(self):
        code, err = self.error_of("--localai-url", "http://127.0.0.1:1")
        self.assertEqual((code, err["components"]), (2, ["localai"]))
        self.assertEqual(err["error"], "LocalAI is not installed.")

    def test_a_localai_that_is_not_ours_is_not_started(self):
        code, err = self.error_of("--localai-url", "http://192.0.2.1:8080",
                                  NOTCHD_LOCALAI_BIN=os.path.join(self.bin, "no-such-localai"))
        self.assertEqual(code, 1)
        self.assertIn("does not answer at http://192.0.2.1:8080", err["error"])
        self.assertNotIn("components", err)

    def test_no_voice_installed_names_the_component(self):
        self.ai.models = ["an-llm"]
        code, err = self.error_of("--localai-url", self.url)
        self.assertEqual((code, err["components"]), (2, ["voice-piper-pt-br"]))
        self.assertEqual(err["error"], "No voice for pt is installed.")
        self.assertEqual(self.ai.requests, [])

    def test_the_voice_of_the_chosen_language_is_the_one_that_speaks(self):
        for code in Voices.languages:
            piper, pocket = (next(v for v in live.known_voices() if v["language"] == code and v["engine"] == e) for e in ("piper", "pocket"))
            self.ai.models = [piper["model"], pocket["model"], "voice-pt_BR-faber-medium"]
            self.ai.requests = []
            self.one_turn("--agent", "codex", "--session", "new", "--voice-lang", code, FAKE_ANSWER="Terminei.")
            self.assertEqual({r["model"] for r in self.ai.requests}, {piper["model"]}, code)
            self.assertEqual(self.ai.requests[0]["input"], piper["warmup"], code)  # the warm-up says something in that language
            self.ai.requests = []
            self.one_turn("--agent", "codex", "--session", "new", "--voice-lang", code, "--voice-engine", "pocket", FAKE_ANSWER="Terminei.")
            self.assertEqual({r["model"] for r in self.ai.requests}, {pocket["model"]}, code)

    def test_a_language_without_its_voice_names_that_languages_component(self):
        self.ai.models = ["voice-pt_BR-faber-medium", "pocket-pt"]  # Portuguese only
        code, err = self.error_of("--localai-url", self.url, "--voice-lang", "en")
        self.assertEqual((code, err["components"]), (2, ["voice-piper-en"]))
        self.assertEqual(err["error"], "No voice for en is installed.")
        code, err = self.error_of("--localai-url", self.url, "--voice-lang", "de", "--voice-engine", "pocket")
        self.assertEqual((code, err["components"]), (2, ["voice-pocket-de"]))
        self.assertEqual(err["error"], "No Pocket TTS voice for de is installed.")
        self.assertEqual(self.ai.requests, [])

    def test_a_chosen_voice_model_that_is_missing(self):
        self.ai.models = ["voice-pt_BR-faber-medium"]
        code, err = self.error_of("--localai-url", self.url, "--voice-model", "pocket-pt")
        self.assertEqual((code, err["components"]), (2, ["voice-pocket-pt"]))
        self.assertIn("pocket-pt isn't installed", err["error"])
        code, err = self.error_of("--localai-url", self.url, "--voice-model", "somebody-elses")
        self.assertEqual(code, 1)
        self.assertNotIn("components", err)

    def test_a_voice_that_does_not_load_stops_the_conversation(self):
        self.ai.fail_warmup = True
        code, err = self.error_of("--localai-url", self.url)
        self.assertEqual(code, 1)
        self.assertEqual(err["error"], "The voice voice-pt_BR-faber-medium did not load: backend crashed")

    def test_a_chosen_model_voice_speech_model_and_language(self):
        answer = "Rodei os testes com sucesso. Terminei tudo por aqui."
        events = self.one_turn("--agent", "codex", "--session", "new", "--voice-model", "pocket-pt", "--voice", "rafael",
                               "--speech-model", "small", "--lang", "pt", FAKE_ANSWER=answer)
        self.assertEqual(self.ai.requests, [
            {"model": "pocket-pt", "input": live.WARMUP_TEXT, "voice": "rafael"},
            {"model": "pocket-pt", "input": "Rodei os testes com sucesso.", "voice": "rafael"},
            {"model": "pocket-pt", "input": "Terminei tudo por aqui.", "voice": "rafael"}])
        # the float WAV at 24 kHz is played as 16-bit at 24 kHz, whole
        self.assertIn("--rate 24000 --channels 1", self.file("pacat-args"))
        self.assertEqual(int(self.file("pacat-bytes").split()[0]), 2 * 6000 * 2)
        self.assertEqual(self.file("voxtype-args").split()[:5], ["-q", "--model", "small", "--language", "pt"])
        self.assertTrue(any(e.get("level", -100) > -60 for e in events))
        self.assertIn("live", [e.get("status") for e in events])
        time.sleep(0.3)
        self.assertEqual(self.leftovers(), [])

    def test_a_failed_sentence_is_skipped_with_a_warning(self):
        events = self.one_turn("--agent", "codex", "--session", "new",
                               FAKE_ANSWER="Isto vai FALHA de verdade. Mas esta fala normalmente.")
        self.assertEqual(len(self.ai.requests), 3)  # the warm-up, the failed one, the next one is still spoken
        self.assertEqual(int(self.file("pacat-bytes").split()[0]), 6000 * 2)
        warnings = [e["warning"] for e in events if e.get("warning")]
        self.assertEqual(warnings, ["A sentence couldn't be spoken: backend crashed"])
        self.assertEqual(self.leftovers(), [])

    def test_localai_started_here_is_stopped_with_the_conversation(self):
        import socket
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        fake = os.path.join(self.dir, "fake-local-ai")
        with open(fake, "w") as f:
            f.write(FAKE_LOCALAI_BINARY)
        os.chmod(fake, 0o755)
        events = self.one_turn("--agent", "codex", "--session", "new", "--localai-url", "http://127.0.0.1:%d" % port,
                               NOTCHD_LOCALAI_BIN=fake)
        argv = self.file("localai-argv").split()
        self.assertEqual(argv[:3], ["run", "--address", "127.0.0.1:%d" % port])
        root = os.path.join(self.dir, "data", "notchd", "localai")
        self.assertEqual(argv[argv.index("--models-path") + 1], os.path.join(root, "models"))
        self.assertEqual(argv[argv.index("--backends-path") + 1], os.path.join(root, "backends"))
        self.assertIn("127.0.0.1", argv[argv.index("--pprof-address") + 1])
        self.assertEqual(self.file("localai-home"), os.path.join(root, "hf"))
        self.assertEqual(self.file("pacat-bytes").split()[0], str(6000 * 2))
        self.assertIn("live", [e.get("status") for e in events])
        pid = int(self.file("localai-pid"))
        time.sleep(0.5)
        self.assertFalse(os.path.exists("/proc/%d" % pid) and "Z" not in open("/proc/%d/stat" % pid).read().rsplit(")", 1)[1][:3],
                         "LocalAI is still running")
        self.assertEqual(self.leftovers(), [])

    def test_localai_already_running_is_not_started_or_stopped(self):
        events = self.one_turn("--agent", "codex", "--session", "new", NOTCHD_LOCALAI_BIN=os.path.join(self.dir, "must-not-run"))
        self.assertFalse(os.path.exists(os.path.join(self.dir, "localai-argv")))
        self.assertTrue(any(r["input"] == live.WARMUP_TEXT for r in self.ai.requests))
        # still answering after the conversation
        self.assertEqual(localai_ready(self.url), True)

    def hang_and_end(self, end, *extra):
        p = self.start("--agent", "codex", "--session", "new", *extra, FAKE_AGENT="hang")
        end_at = time.time() + 20
        while not os.path.exists(os.path.join(self.dir, "agent-started")):
            self.assertLess(time.time(), end_at, "the agent never started")
            time.sleep(0.05)
        self.assertTrue(self.leftovers())
        end(p)
        self.assertIn(p.wait(timeout=15), (0, 124 + 0, -signal.SIGTERM, 143))
        time.sleep(0.5)
        self.assertEqual(self.leftovers(), [])

    def test_stop_kills_the_agent(self):
        def end(p):
            p.stdin.write('{"stop": true}\n')
            p.stdin.flush()
        self.hang_and_end(end)

    def test_end_of_stdin_kills_the_agent(self):
        self.hang_and_end(lambda p: p.stdin.close())

    def test_sigterm_kills_the_agent(self):
        def end(p):
            out = subprocess.run(["pgrep", "-P", str(p.pid)], capture_output=True, text=True).stdout.split()
            # `timeout` is p; its child is the bridge
            os.kill(int(out[0]), signal.SIGTERM)
        self.hang_and_end(end)

    def test_mute_is_answered(self):
        p = self.start("--agent", "codex", "--session", "new")
        self.wait_for(lambda e: e.get("status") == "live")
        p.stdin.write('{"mute": true}\n')
        p.stdin.flush()
        self.wait_for(lambda e: e.get("muted") is True)
        p.stdin.write('{"stop": true}\n')
        p.stdin.flush()
        self.assertEqual(p.wait(timeout=15), 0)


if __name__ == "__main__":
    unittest.main()
