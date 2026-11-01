#!/usr/bin/env python3
"""A spoken conversation with a coding agent, for notchd (core/LiveModule.qml),
with speech-to-text and text-to-speech that run on this machine.

    local-live.py [--agent codex|claude] [--command <command>]
                  [--session last|new|<id>] [--cwd <folder>]
                  [--access read-only|workspace-write]
                  [--whisper-model <ggml file>] [--voice <piper .onnx>]
                  [--lang pt] [--mic default|<file.wav>] [--log <file>]

The cascade, as opposed to codex-live.py's Codex realtime voice: the
microphone (parec) goes through a voice activity detector; each utterance
is written to a WAV and transcribed by whisper.cpp (whisper-cli); the words
go into a NORMAL text session of the agent (`codex exec resume`, `claude -p
--resume`), the same conversation you type in; its answer, as it streams,
is cut into sentences, each spoken by Piper and played with pacat, the next
one synthesized while the last plays.

--session says which: "last" (the default) the most recent session in --cwd,
"new" one of its own (its id kept for the turns after), or an id.

stdout, the protocol codex-live.py documents, one JSON object a line:

  {"status": "starting" | "connecting" | "live" | "closed", "reason"?}
  {"transcript": {"role": "user" | "assistant", "delta": "..."}}
  {"transcript": {"role": ..., "text": "...", "done": true}}
  {"level": <dB>}          the voice as it plays, 25 a second; -100 silent
  {"mic": <dB>}            what the microphone hears, 10 a second
  {"working": true|false}  a turn is in flight (the agent, then its voice)
  {"muted": true|false}
  {"warning": "..."}
  {"error": "...", "packages": [...]?}   then it ends, exit code 2 when
                                         something is missing, else 1

stdin: {"mute": true|false}, {"stop": true}; end of stdin or SIGTERM stops it.

Half duplex, for now: while a turn is in flight (the agent works, the voice
plays) the microphone is ignored, there being no echo cancellation. Barge-in
(speaking over it stops it) is a follow-up. Every child (parec, whisper-cli,
the agent, piper, pacat) is killed when it ends.

--mic takes a 16 kHz mono 16-bit WAV in the microphone's place, for trying it
without speaking; NOTCHD_LIVE_MIC_SPEED=<n> plays it n times faster (tests).
Binaries are looked up on PATH; NOTCHD_WHISPER_CLI and NOTCHD_PIPER name
others. Models default to $XDG_DATA_HOME/notchd/models/ (`make live-models`).
The log is by default $XDG_STATE_HOME/notchd/local-live.log.

The rules that read these lines are core/Live.js's. Needs libpulse (parec,
pacat), whisper-cpp, Piper (PyPI's piper-tts, `piper` on PATH or in
~/.local/bin) and the agent logged in (`make local-live-deps`).
"""

import argparse
import array
import json
import math
import os
import queue
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import wave

MIC_RATE = 16000
FRAME_S = 0.05
FRAME_BYTES = int(MIC_RATE * FRAME_S) * 2
# Said with every spoken turn, so the answer comes out speakable. Change here.
SPOKEN_HINT = ("[Voice conversation: the user is speaking and your answer is read aloud. "
               "Answer briefly, in short plain sentences, in the language the user spoke. "
               "No markdown, lists, tables or code blocks; say what you did rather than showing code.]")
# Silence after the voice before the microphone is heard again.
TAIL_S = 0.4
AGENT_TIMEOUT = 600
LEVEL_S = 0.04
LEAD_S = 0.15
DEBUG = os.environ.get("NOTCHD_LIVE_DEBUG") == "1"

_out_lock = threading.Lock()
_log_file = None


def log(text):
    line = "%s %s\n" % (time.strftime("%H:%M:%S"), text)
    if _log_file:
        try:
            _log_file.write(line)
            _log_file.flush()
        except OSError:
            pass
    if DEBUG:
        sys.stderr.write(line)


def emit(obj):
    if "level" not in obj and "mic" not in obj:
        log("-> %s" % json.dumps(obj, ensure_ascii=False))
    with _out_lock:
        try:
            sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
            sys.stdout.flush()
        except (OSError, ValueError):
            pass


# ------------------------------------------------------------- pure rules

def rms_db(data):
    """The level of 16-bit little-endian samples, dB below full scale (0 is
    full scale); -100 for silence."""
    n = len(data) // 2
    if n == 0:
        return -100.0
    samples = array.array("h")
    samples.frombytes(data[:n * 2])
    if sys.byteorder == "big":
        samples.byteswap()
    total = 0
    for s in samples:
        total += s * s
    rms = math.sqrt(total / n) / 32768.0
    if rms <= 0:
        return -100.0
    return max(-100.0, round(20 * math.log10(rms), 1))


class Vad:
    """Voice activity from frame levels (dB). The noise floor is the 10th
    percentile of the last `window` seconds (frames of digital silence,
    -100 dB, left out: a mic waking up, or noise suppression; with only
    those, the floor is -100), followed all the time, during speech too:
    noise that starts while you talk (music, a fan) raises it, and the
    utterance still ends, some seconds late. It falls at once and rises at
    most `rise_db` a second, so speech alone (after digital silence) doesn't
    become the floor. Speech starts over floor + `margin_db` and goes on
    while over `hysteresis_db` less. `feed(db)` returns "start" when speech begins, "end"
    when it has been over for `hangover` seconds (after at least `min_utt` of
    voiced frames) and "drop" when what ended was too short to be words;
    else None. Nothing in the first `calib` seconds of sound."""

    def __init__(self, frame_s=FRAME_S, hangover=0.7, min_utt=0.3, calib=0.5,
                 margin_db=12.0, hysteresis_db=6.0, min_threshold=-72.0,
                 max_utt=20.0, window=4.0, rise_db=10.0):
        self.frame_s = frame_s
        self.hangover = hangover
        self.min_utt = min_utt
        self.calib = calib
        self.margin_db = margin_db
        self.hysteresis_db = hysteresis_db
        self.min_threshold = min_threshold
        self.max_utt = max_utt
        self.window = max(1, int(round(window / frame_s)))
        self.rise_db = rise_db
        self.reset(full=True)

    def reset(self, full=False):
        if full:
            self.floor = None
            self.history = []
            self.elapsed = 0.0
        self.speaking = False
        self.silence = 0.0
        self.voiced = 0.0
        self.length = 0.0

    @property
    def ready(self):
        return self.floor is not None

    @property
    def threshold(self):
        """The level speech starts over."""
        if self.floor is None:
            return 0.0
        return max(self.floor + self.margin_db, self.min_threshold)

    @property
    def end_threshold(self):
        """The level speech goes on over, once started."""
        return self.threshold - self.hysteresis_db

    def _follow(self, db):
        self.elapsed += self.frame_s
        if db > -99:
            self.history = (self.history + [db])[-self.window:]
        if self.floor is None and self.elapsed < self.calib - 1e-9:
            return
        target = sorted(self.history)[len(self.history) // 10] if self.history else -100.0
        if self.floor is None or target < self.floor:
            self.floor = target
        else:
            self.floor = min(target, self.floor + self.rise_db * self.frame_s)

    def feed(self, db):
        was_ready = self.floor is not None
        self._follow(db)
        if not was_ready:
            return None
        if not self.speaking:
            if db > self.threshold:
                self.speaking = True
                self.silence = 0.0
                self.voiced = self.frame_s
                self.length = self.frame_s
                return "start"
            return None
        self.length += self.frame_s
        if db > self.end_threshold:
            self.silence = 0.0
            self.voiced += self.frame_s
        else:
            self.silence += self.frame_s
        if self.silence >= self.hangover - 1e-9 or self.length >= self.max_utt:
            ok = self.voiced >= self.min_utt - 1e-9
            self.reset()
            return "end" if ok else "drop"
        return None


_SENTENCE_END = re.compile(r"[.!?…]+[\"')\]]*(?:\s+|$)|\n+")


def split_sentences(buffer, final=False, min_len=24):
    """The text of an answer as it arrives -> (sentences ready, the rest).
    A sentence ends at . ! ? … followed by space, or at a line break; ones
    shorter than `min_len` join the next (so "Ok." isn't a request of its
    own). With `final`, the rest is a sentence too."""
    sentences = []
    start = 0
    pending = ""
    for m in _SENTENCE_END.finditer(buffer):
        if m.end() == len(buffer) and not final and not m.group(0)[-1:].isspace():
            break  # a stop at the very end may be "3." of "3.5": wait for more
        piece = pending + buffer[start:m.end()]
        start = m.end()
        if len(piece.strip()) < min_len:
            pending = piece
            continue
        sentences.append(piece.strip())
        pending = ""
    rest = pending + buffer[start:]
    if final:
        if rest.strip():
            sentences.append(rest.strip())
        rest = ""
    return [s for s in sentences if s], rest


def strip_markdown(text):
    """Text to be read aloud: no code blocks, links, emphasis, headings,
    bullets, tables or URLs."""
    t = re.sub(r"```.*?(```|$)", "\n", text, flags=re.S)
    t = re.sub(r"`([^`]*)`", r"\1", t)
    t = re.sub(r"!\[([^\]]*)\]\([^)]*\)", r"\1", t)
    t = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", t)
    t = re.sub(r"https?://\S+", "", t)
    t = re.sub(r"(?m)^\s{0,3}#{1,6}\s*", "", t)
    t = re.sub(r"(?m)^\s*(?:[-*+•]|\d+[.)])\s+", "", t)
    t = re.sub(r"(?m)^\s*>\s?", "", t)
    t = re.sub(r"(?m)^\s*\|?\s*:?-{3,}.*$", "", t)
    t = t.replace("|", ", ")
    t = re.sub(r"(\*\*|__|\*|~~)", "", t)
    t = re.sub(r"(?<![\w])_|_(?![\w])", "", t)
    t = re.sub(r"[ \t]+", " ", t)
    t = re.sub(r"[ \t]*\n\s*", "\n", t)
    return t.strip()


def normalize(pcm, peak=0.7, max_gain=30.0):
    """16-bit PCM scaled so its loudest sample is `peak` of full scale, at
    most `max_gain` times: a quiet microphone (its volume low in the
    mixer) still gives whisper words it can hear."""
    samples = array.array("h")
    samples.frombytes(pcm[:len(pcm) // 2 * 2])
    if sys.byteorder == "big":
        samples.byteswap()
    top = max((abs(x) for x in samples), default=0)
    if top == 0:
        return pcm
    gain = min(max_gain, peak * 32767 / top)
    if gain <= 1.0:
        return pcm
    out = array.array("h", (max(-32768, min(32767, int(x * gain))) for x in samples))
    if sys.byteorder == "big":
        out.byteswap()
    return out.tobytes()


def clean_transcript(text):
    """whisper-cli's stdout (no timestamps) -> the words: annotations such as
    [BLANK_AUDIO] or (music) dropped, lines joined."""
    t = re.sub(r"\[[^\]]*\]|\([^)]*\)|\*[^*]*\*", " ", text)
    t = " ".join(t.split())
    return t.strip()


def sandbox_for(access):
    return "workspace-write" if access == "workspace-write" else "read-only"


def agent_argv(agent, command, session, cwd, access):
    """The agent's command line for one turn; the prompt goes on stdin.
    `session` is "new" (a session of its own), "last" (the most recent in
    `cwd`) or an id. `command` is the user's command (shell words)."""
    base = shlex.split(command) if command and command.strip() else [agent]
    if agent == "claude":
        argv = base + ["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages",
                       "--permission-mode", "acceptEdits" if access == "workspace-write" else "plan"]
        if session == "last":
            argv += ["--continue"]
        elif session != "new":
            argv += ["--resume", session]
        return argv
    argv = base + ["exec", "--json", "--skip-git-repo-check", "-s", sandbox_for(access), "-C", cwd]
    if session == "last":
        argv += ["resume", "--last"]
    elif session != "new":
        argv += ["resume", session]
    return argv + ["-"]


def parse_agent_line(agent, line):
    """One line of the agent's JSON stream -> [event]: ("session", id),
    ("delta", text) text as it comes, ("message", text) a whole message,
    ("break",) between messages, ("done",), ("error", text)."""
    try:
        e = json.loads(line)
    except (ValueError, TypeError):
        return []
    if not isinstance(e, dict):
        return []
    kind = e.get("type")
    out = []
    if agent == "claude":
        sid = e.get("session_id")
        if kind in ("system", "result") and isinstance(sid, str) and sid:
            out.append(("session", sid))
        if kind == "stream_event":
            ev = e.get("event") or {}
            if ev.get("type") == "content_block_delta":
                d = ev.get("delta") or {}
                if d.get("type") == "text_delta" and d.get("text"):
                    out.append(("delta", d["text"]))
            elif ev.get("type") == "content_block_stop":
                out.append(("break",))
        elif kind == "result":
            if e.get("is_error"):
                out.append(("error", str(e.get("result") or "the agent failed")))
            out.append(("done",))
        return out
    if kind in ("thread.started",) and isinstance(e.get("thread_id"), str):
        out.append(("session", e["thread_id"]))
    elif kind == "item.completed":
        item = e.get("item") or {}
        if item.get("type") == "agent_message" and item.get("text"):
            out.append(("message", str(item["text"])))
    elif kind == "turn.completed":
        out.append(("done",))
    elif kind in ("turn.failed", "error"):
        err = e.get("error") if isinstance(e.get("error"), dict) else {}
        out.append(("error", str(err.get("message") or e.get("message") or "the agent failed")))
    return out


def parse_command(line):
    try:
        c = json.loads(line)
    except ValueError:
        return None
    if not isinstance(c, dict):
        return None
    if c.get("stop"):
        return ("stop", None)
    if isinstance(c.get("mute"), bool):
        return ("mute", c["mute"])
    return None


def default_models(data_home, home):
    base = data_home or os.path.join(home, ".local", "share")
    return os.path.join(base, "notchd", "models")


def first_existing(folder, names):
    for n in names:
        p = os.path.join(folder, n)
        if os.path.isfile(p):
            return p
    return ""


def voice_rate(onnx_path):
    """The voice's sample rate, from the .onnx.json next to it."""
    try:
        with open(onnx_path + ".json", encoding="utf-8") as f:
            return int(json.load(f)["audio"]["sample_rate"])
    except (OSError, ValueError, KeyError, TypeError):
        return 22050


def work_dir(cwd, home):
    c = (cwd or "").strip()
    if c == "~" or c.startswith("~/"):
        c = home + c[1:]
    return c if c else os.path.join(home, "Documents", "notchd")


# -------------------------------------------------------------- processes

class Children:
    """Every child process: killed together, whatever ends the bridge."""

    def __init__(self):
        self.lock = threading.Lock()
        self.procs = set()
        self.closed = False

    def spawn(self, argv, **kw):
        with self.lock:
            if self.closed:
                raise RuntimeError("closing")
            p = subprocess.Popen(argv, start_new_session=True, **kw)
            self.procs.add(p)
        return p

    def done(self, p):
        with self.lock:
            self.procs.discard(p)

    def kill(self, p, wait=2.0):
        if p.poll() is None:
            try:
                os.killpg(p.pid, signal.SIGTERM)
            except OSError:
                pass
            try:
                p.wait(timeout=wait)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(p.pid, signal.SIGKILL)
                except OSError:
                    pass
                try:
                    p.wait(timeout=wait)
                except subprocess.TimeoutExpired:
                    pass
        self.done(p)

    def kill_all(self):
        with self.lock:
            self.closed = True
            procs = list(self.procs)
        for p in procs:
            if p.poll() is None:
                try:
                    os.killpg(p.pid, signal.SIGTERM)
                except OSError:
                    pass
        end = time.time() + 1.5
        for p in procs:
            try:
                p.wait(timeout=max(0.05, end - time.time()))
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(p.pid, signal.SIGKILL)
                except OSError:
                    pass
                try:
                    p.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    pass


def find_binary(env_name, names):
    override = os.environ.get(env_name)
    if override:
        return override
    for n in names:
        p = shutil.which(n)
        if p:
            return p
    # uv and pipx install into ~/.local/bin, which a desktop's PATH may lack.
    for n in names:
        p = os.path.join(os.path.expanduser("~"), ".local", "bin", n)
        if os.access(p, os.X_OK):
            return p
    return ""


def missing_requirements(args, models):
    """(message, packages) of what the conversation can't run without, or
    None."""
    packages = []
    notes = []
    if not shutil.which("parec") and args.mic == "default":
        packages.append("libpulse")
    if not shutil.which("pacat"):
        packages.append("libpulse")
    if not find_binary("NOTCHD_WHISPER_CLI", ["whisper-cli", "whisper-cpp"]):
        packages.append("whisper-cpp")
    if not find_binary("NOTCHD_PIPER", ["piper", "piper-tts"]):
        packages.append("piper-tts")
    packages = [p for i, p in enumerate(packages) if p not in packages[:i]]
    agent_cmd = (shlex.split(args.command)[0] if args.command.strip() else args.agent)
    if not shutil.which(agent_cmd):
        notes.append("%s isn't installed." % agent_cmd)
    if not models["whisper"] or not os.path.isfile(models["whisper"]):
        notes.append("Speech model not found (%s)." % (models["whisper"] or "none in " + models["dir"]))
    if not models["voice"] or not os.path.isfile(models["voice"]):
        notes.append("Voice not found (%s)." % (models["voice"] or "none in " + models["dir"]))
    if not packages and not notes:
        return None
    msg = "Voice needs " + ("packages that aren't installed. " if packages else "") + " ".join(notes)
    if packages:
        msg += " Install " + ", ".join(packages) + " (make local-live-deps)."
    if any("model" in n or "Voice not" in n for n in notes):
        msg += " (make live-models)"
    return msg.strip(), packages


# ----------------------------------------------------------------- speech

class Speaker:
    """One turn's voice: sentences in, synthesized by Piper one after the
    other while the last plays through pacat; the level of what is played."""

    def __init__(self, bridge):
        self.b = bridge
        self.rate = voice_rate(bridge.voice)
        self.buffer = ""
        self.texts = queue.Queue()
        self.audio = queue.Queue(maxsize=3)
        self.started = False
        self.synth_t = threading.Thread(target=self.synth, daemon=True)
        self.play_t = threading.Thread(target=self.play, daemon=True)
        self.synth_t.start()
        self.play_t.start()

    def feed(self, text):
        self.buffer += text
        ready, self.buffer = split_sentences(self.buffer)
        for s in ready:
            self.say(s)

    def flush(self):
        """A message ends: what is left is a sentence."""
        ready, self.buffer = split_sentences(self.buffer, final=True)
        for s in ready:
            self.say(s)

    def say(self, sentence):
        t = strip_markdown(sentence)
        if re.search(r"\w", t):
            self.texts.put(" ".join(t.split()))

    def finish(self):
        self.flush()
        self.texts.put(None)
        self.synth_t.join()
        self.play_t.join()

    def synth(self):
        b = self.b
        piper = find_binary("NOTCHD_PIPER", ["piper", "piper-tts"])
        while True:
            text = self.texts.get()
            if text is None or b.stop.is_set():
                break
            log("tts: %s" % text)
            try:
                p = b.children.spawn([piper, "--model", b.voice, "--output-raw"], stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=b.log_fd)
                try:
                    out, _ = p.communicate((text + "\n").encode("utf-8"), timeout=120)
                except subprocess.TimeoutExpired:
                    b.children.kill(p)
                    out = b""
                b.children.done(p)
            except (OSError, RuntimeError) as e:
                log("piper failed: %s" % e)
                break
            if out:
                self.audio.put(out)
        self.audio.put(None)

    def play(self):
        b = self.b
        pacat = None
        sent = 0.0
        t0 = 0.0
        chunk = max(2, int(self.rate * LEVEL_S) * 2)
        try:
            while True:
                data = self.audio.get()
                if data is None or b.stop.is_set():
                    break
                if pacat is None:
                    pacat = b.children.spawn(["pacat", "--raw", "--format", "s16le", "--rate", str(self.rate),
                                              "--channels", "1", "--latency-msec", "100"],
                                             stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=b.log_fd)
                    t0 = time.time()
                    b.speaking = True
                for i in range(0, len(data), chunk):
                    if b.stop.is_set():
                        return
                    piece = data[i:i + chunk]
                    # Hold the pipe to what plays: the level is what is heard.
                    wait = t0 + sent - LEAD_S - time.time()
                    if wait > 0:
                        b.stop.wait(wait)
                    try:
                        pacat.stdin.write(piece)
                        pacat.stdin.flush()
                    except (OSError, ValueError):
                        return
                    emit({"level": rms_db(piece)})
                    sent += len(piece) / 2.0 / self.rate
            if pacat is not None:
                try:
                    pacat.stdin.close()
                except OSError:
                    pass
                b.stop.wait(max(0.0, t0 + sent - time.time()))
        finally:
            if pacat is not None:
                b.children.kill(pacat, wait=3.0)
            emit({"level": -100})
            b.speaking = False


# ----------------------------------------------------------------- bridge

class Bridge:
    def __init__(self, args, models, log_fd):
        self.args = args
        self.voice = models["voice"]
        self.whisper = models["whisper"]
        self.stop = threading.Event()
        self.children = Children()
        self.muted = False
        self.speaking = False
        self.busy = False
        self.tail_until = 0.0
        self.exit_code = 0
        self.log_fd = log_fd
        self.session = args.session
        self.next_session_id = None
        self.utterances = queue.Queue()
        self.tmp = tempfile.mkdtemp(prefix="notchd-live-")
        self.home = os.path.expanduser("~")
        self.cwd = work_dir(args.cwd, self.home)

    # ------------------------------------------------------------ control
    def fail(self, message, packages=None, code=None):
        ev = {"error": message}
        if packages:
            ev["packages"] = packages
        emit(ev)
        self.exit_code = code if code is not None else (2 if packages else 1)
        self.stop.set()

    def read_commands(self):
        for line in sys.stdin:
            cmd = parse_command(line)
            if not cmd:
                continue
            if cmd[0] == "stop":
                break
            self.muted = cmd[1]
            emit({"muted": self.muted})
        self.stop.set()

    # ---------------------------------------------------------------- mic
    def mic_frames(self):
        """Frames of 16 kHz mono s16le, from parec or the WAV."""
        if self.args.mic == "default":
            p = self.children.spawn(["parec", "--format=s16le", "--rate=%d" % MIC_RATE, "--channels=1",
                                     "--latency-msec=50"], stdout=subprocess.PIPE, stderr=self.log_fd)
            try:
                while not self.stop.is_set():
                    buf = p.stdout.read(FRAME_BYTES)
                    if len(buf) < FRAME_BYTES:
                        if not self.stop.is_set():
                            self.fail("The microphone stopped (parec ended).")
                        return
                    yield buf
            finally:
                self.children.kill(p)
            return
        speed = float(os.environ.get("NOTCHD_LIVE_MIC_SPEED", "1") or 1)
        with wave.open(self.args.mic, "rb") as w:
            if (w.getframerate(), w.getnchannels(), w.getsampwidth()) != (MIC_RATE, 1, 2):
                self.fail("--mic needs a 16 kHz mono 16-bit WAV.")
                return
            data = w.readframes(w.getnframes())
        silence = b"\0" * FRAME_BYTES
        pos = 0
        while not self.stop.is_set():
            if pos < len(data):
                buf = data[pos:pos + FRAME_BYTES].ljust(FRAME_BYTES, b"\0")
                pos += FRAME_BYTES
            else:
                buf = silence
            yield buf
            self.stop.wait(FRAME_S / speed)

    def mic_loop(self):
        vad = Vad()
        pre = []
        utt = []
        n = 0
        live = False
        for frame in self.mic_frames():
            n += 1
            db = -100.0 if self.muted else rms_db(frame)
            if n % 2 == 0:
                emit({"mic": db})
            if self.muted or self.busy or time.time() < self.tail_until:
                if vad.speaking:
                    vad.reset()
                utt, pre = [], []
                continue
            ev = vad.feed(db)
            if n % 40 == 0:
                log("mic %.1f dB, floor %s, threshold %.1f, %s" % (
                    db, "-" if vad.floor is None else "%.1f" % vad.floor, vad.threshold,
                    "speaking" if vad.speaking else "quiet"))
            if ev:
                log("vad %s (floor %.1f, threshold %.1f)" % (ev, vad.floor, vad.threshold))
            if vad.ready and not live:
                live = True
                emit({"status": "live"})
            if ev == "start":
                utt = pre + [frame]
            elif vad.speaking:
                utt.append(frame)
            if ev == "end":
                utt.append(frame)
                emit({"working": True})
                self.busy = True
                self.utterances.put(b"".join(utt))
                utt = []
            if ev in ("end", "drop"):
                utt = []
            pre = (pre + [frame])[-6:]

    # --------------------------------------------------------------- turn
    def transcribe(self, pcm):
        path = os.path.join(self.tmp, "utterance.wav")
        with wave.open(path, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(MIC_RATE)
            w.writeframes(normalize(pcm))
        exe = find_binary("NOTCHD_WHISPER_CLI", ["whisper-cli", "whisper-cpp"])
        p = self.children.spawn([exe, "-m", self.whisper, "-f", path, "-l", self.args.lang, "-nt", "-np"],
                                stdout=subprocess.PIPE, stderr=self.log_fd, stdin=subprocess.DEVNULL)
        try:
            out, _ = p.communicate(timeout=120)
        except subprocess.TimeoutExpired:
            self.children.kill(p)
            return ""
        self.children.done(p)
        try:
            os.unlink(path)
        except OSError:
            pass
        return clean_transcript(out.decode("utf-8", "replace"))

    def run_agent(self, text, speaker):
        """One turn on the agent's session; its answer streams to the
        transcript and the voice. Returns the whole answer."""
        argv = agent_argv(self.args.agent, self.args.command, self.session, self.cwd, self.args.access)
        log("agent: %s" % " ".join(shlex.quote(a) for a in argv))
        os.makedirs(self.cwd, exist_ok=True)
        p = self.children.spawn(argv, cwd=self.cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=self.log_fd)
        timer = threading.Timer(AGENT_TIMEOUT, lambda: self.children.kill(p))
        timer.daemon = True
        timer.start()
        answer = ""
        failed = ""
        try:
            try:
                p.stdin.write((SPOKEN_HINT + "\n\n" + text + "\n").encode("utf-8"))
                p.stdin.close()
            except OSError:
                pass
            for raw in p.stdout:
                if self.stop.is_set():
                    break
                for ev in parse_agent_line(self.args.agent, raw.decode("utf-8", "replace")):
                    kind = ev[0]
                    if kind == "session":
                        if self.session != ev[1]:
                            log("session: %s" % ev[1])
                        self.session = ev[1]
                    elif kind in ("delta", "message"):
                        piece = ev[1]
                        if kind == "message" and answer:
                            piece = "\n" + piece
                        answer += piece
                        emit({"transcript": {"role": "assistant", "delta": piece}})
                        speaker.feed(ev[1] + ("\n" if kind == "message" else ""))
                    elif kind == "break":
                        speaker.flush()
                        if answer and not answer.endswith("\n"):
                            answer += "\n"
                            emit({"transcript": {"role": "assistant", "delta": "\n"}})
                    elif kind == "error":
                        failed = ev[1]
        finally:
            timer.cancel()
            self.children.kill(p)
        if failed and not self.stop.is_set():
            emit({"warning": "The agent failed: " + failed[:200]})
        return answer.strip()

    def turn(self, pcm):
        try:
            text = self.transcribe(pcm)
            if not text:
                return
            emit({"working": True})
            emit({"transcript": {"role": "user", "text": text, "done": True}})
            speaker = Speaker(self)
            try:
                answer = self.run_agent(text, speaker)
            finally:
                speaker.finish()
            if answer:
                emit({"transcript": {"role": "assistant", "text": answer, "done": True}})
        except (OSError, RuntimeError) as e:
            if not self.stop.is_set():
                emit({"warning": "A turn failed: %s" % e})
        finally:
            emit({"working": False})
            self.tail_until = time.time() + TAIL_S
            self.busy = False

    def worker(self):
        while not self.stop.is_set():
            try:
                pcm = self.utterances.get(timeout=0.2)
            except queue.Empty:
                continue
            self.turn(pcm)

    # ---------------------------------------------------------------- run
    def run(self):
        threading.Thread(target=self.read_commands, daemon=True).start()
        emit({"status": "connecting"})
        threading.Thread(target=self.worker, daemon=True).start()
        threading.Thread(target=self.mic_loop, daemon=True).start()
        self.stop.wait()
        self.shutdown()
        return self.exit_code

    def shutdown(self):
        self.children.kill_all()
        shutil.rmtree(self.tmp, ignore_errors=True)
        emit({"status": "closed", "reason": "requested" if not self.exit_code else "error"})


def main(argv=None):
    global _log_file
    home = os.path.expanduser("~")
    parser = argparse.ArgumentParser(description="A spoken conversation with an agent, locally.")
    parser.add_argument("--agent", choices=["codex", "claude"], default="codex")
    parser.add_argument("--command", default="")
    parser.add_argument("--session", default="last")
    parser.add_argument("--cwd", default="")
    parser.add_argument("--access", default="read-only")
    parser.add_argument("--whisper-model", default="")
    parser.add_argument("--voice", default="")
    parser.add_argument("--lang", default="pt")
    parser.add_argument("--mic", default="default")
    parser.add_argument("--log", default=os.path.join(
        os.environ.get("XDG_STATE_HOME") or os.path.join(home, ".local", "state"), "notchd", "local-live.log"))
    args = parser.parse_args(argv)
    args.session = args.session.strip() or "last"

    log_fd = subprocess.DEVNULL
    try:
        os.makedirs(os.path.dirname(args.log), exist_ok=True)
        _log_file = open(args.log, "a", encoding="utf-8")
        log_fd = _log_file
    except OSError:
        pass
    log("local-live starting: %s" % " ".join(sys.argv[1:]))

    folder = default_models(os.environ.get("XDG_DATA_HOME", ""), home)
    models = {
        "dir": folder,
        "whisper": os.path.expanduser(args.whisper_model) if args.whisper_model else first_existing(
            folder, ["ggml-small.bin", "ggml-medium.bin", "ggml-base.bin", "ggml-small.en.bin"]),
        "voice": os.path.expanduser(args.voice) if args.voice else first_existing(
            folder, ["pt_BR-faber-medium.onnx", "pt_BR-cadu-medium.onnx", "pt_BR-edresson-low.onnx"]),
    }
    emit({"status": "starting"})
    problem = missing_requirements(args, models)
    if problem:
        emit({"error": problem[0], **({"packages": problem[1]} if problem[1] else {})})
        return 2 if problem[1] else 1

    bridge = Bridge(args, models, log_fd)

    def on_signal(signum, frame):
        bridge.stop.set()
    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGHUP, on_signal)
    try:
        return bridge.run()
    finally:
        bridge.children.kill_all()


if __name__ == "__main__":
    sys.exit(main())
