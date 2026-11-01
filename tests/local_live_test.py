"""Tests of core/live/local-live.py, the cascade bridge (whisper.cpp, an
agent's text session, Piper): the rules (voice activity, sentences,
markdown, the agent's command line and stream) and the whole bridge run with
fake whisper-cli, piper, pacat, codex and claude on PATH and a WAV for the
microphone. Nothing here touches your config, your sessions or the sound.
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

    def test_whisper_output(self):
        self.assertEqual(live.clean_transcript(" [BLANK_AUDIO]\n"), "")
        self.assertEqual(live.clean_transcript(" Olá, tudo bem?\n (música) \n"), "Olá, tudo bem?")


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
        self.assertEqual(live.default_models("", "/h"), "/h/.local/share/notchd/models")
        self.assertEqual(live.default_models("/d", "/h"), "/d/notchd/models")
        self.assertEqual(live.work_dir("", "/h"), "/h/Documents/notchd")
        self.assertEqual(live.work_dir("~/code", "/h"), "/h/code")

    def test_voice_rate(self):
        with tempfile.TemporaryDirectory() as d:
            v = os.path.join(d, "v.onnx")
            self.assertEqual(live.voice_rate(v), 22050)
            with open(v + ".json", "w") as f:
                json.dump({"audio": {"sample_rate": 16000}}, f)
            self.assertEqual(live.voice_rate(v), 16000)


FAKES = {
    "whisper-cli": """#!/usr/bin/env python3
import os, sys
open(os.path.join(os.environ["FAKE_DIR"], "whisper-args"), "a").write(" ".join(sys.argv[1:]) + "\\n")
print(" Rode os testes, por favor.")
""",
    "piper": """#!/usr/bin/env python3
import os, struct, sys, math
text = sys.stdin.read()
open(os.path.join(os.environ["FAKE_DIR"], "piper-text"), "a").write(text)
n = 16000 // 4
sys.stdout.buffer.write(b"".join(struct.pack("<h", int(8000 * math.sin(i / 5))) for i in range(n)))
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
    print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": "**Rodei** os testes. Todos passaram."}}), flush=True)
    print(json.dumps({"type": "turn.completed"}), flush=True)
""",
}


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
        self.whisper = os.path.join(self.dir, "ggml-small.bin")
        self.voice = os.path.join(self.dir, "voice.onnx")
        open(self.whisper, "w").close()
        open(self.voice, "w").close()
        with open(self.voice + ".json", "w") as f:
            json.dump({"audio": {"sample_rate": 16000}}, f)
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
        cmd = ["timeout", "-k", "2", "40", sys.executable, SCRIPT, "--mic", self.wav, "--whisper-model", self.whisper,
               "--voice", self.voice, "--cwd", self.work, "--lang", "pt"] + list(args)
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
        self.assertIn("-l pt -nt", self.file("whisper-args"))
        call = self.lines("agent-calls")[0]
        self.assertEqual(call["argv"][:2], ["exec", "--json"])
        self.assertNotIn("resume", call["argv"])
        self.assertIn("Rode os testes, por favor.", call["stdin"])
        self.assertIn(live.SPOKEN_HINT, call["stdin"])
        self.assertEqual(os.path.realpath(call["cwd"]), os.path.realpath(self.work))
        # the answer was spoken, markdown stripped, sentence by sentence
        self.assertEqual(self.file("piper-text").splitlines(), ["Rodei os testes. Todos passaram."])
        self.assertIn("--rate 16000 --channels 1", self.file("pacat-args"))
        self.assertGreater(int(self.file("pacat-bytes").split()[0]), 1000)
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
        p = subprocess.run([sys.executable, SCRIPT, "--mic", self.wav, "--whisper-model", self.whisper, "--voice", self.voice],
                           capture_output=True, text=True, timeout=20,
                           env={"PATH": "/nonexistent", "HOME": self.dir, "XDG_STATE_HOME": self.dir})
        self.assertEqual(p.returncode, 2)
        err = [json.loads(l) for l in p.stdout.splitlines() if "error" in l][0]
        self.assertEqual(sorted(err["packages"]), ["libpulse", "piper-tts", "whisper-cpp"])

    def test_missing_models_say_so(self):
        p = subprocess.run([sys.executable, SCRIPT, "--mic", self.wav, "--whisper-model", "/nope/x.bin", "--voice", "/nope/v.onnx"],
                           capture_output=True, text=True, timeout=20, env=self.env())
        self.assertEqual(p.returncode, 1)
        err = [json.loads(l) for l in p.stdout.splitlines() if "error" in l][0]
        self.assertIn("make live-models", err["error"])
        self.assertNotIn("packages", err)

    def hang_and_end(self, end):
        p = self.start("--agent", "codex", "--session", "new", FAKE_AGENT="hang")
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
