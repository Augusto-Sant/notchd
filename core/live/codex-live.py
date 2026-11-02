#!/usr/bin/env python3
"""A spoken conversation with Codex, for notchd (core/LiveModule.qml).

    codex-live.py [--codex <command>] [--cwd <folder>] [--voice <name>]
                  [--access read-only|workspace-write] [--mic default|none]

Starts Codex's own server (`codex app-server`), makes a thread of its own
in --cwd (notchd's folder in your Documents without one) and opens a
realtime conversation on it: version v3 over WebRTC, the one a ChatGPT
login may use (over a websocket Codex asks for an API key). The voice
comes and goes over WebRTC (GStreamer's webrtcbin): the microphone in, with
the echo of the speakers taken out (webrtcdsp), and Codex's voice out to
the speakers. What it hears and says, and its tasks, come from the server.

stdout, one JSON object a line:

  {"status": "starting" | "connecting" | "live" | "closed", "reason"?}
  {"transcript": {"role": "user" | "assistant", "delta": "..."}}
  {"transcript": {"role": ..., "text": "...", "done": true}}
  {"level": <dB>}          Codex's voice as it plays, 25 a second; -100 silent
  {"mic": <dB>}            what the microphone sends, 10 a second; -100 silent
  {"working": true|false}  Codex works on a task it was given
  {"muted": true|false}
  {"warning": "..."}
  {"error": "...", "packages": [...]?, "components": [...]?}
                           then it ends, exit code 2 when packages are
                           missing, else 1; `components`: ids of
                           core/components/components.json that install them

stdin, one JSON object a line: {"mute": true|false}, {"stop": true}. End
of stdin or SIGTERM stops it too.

--mic takes a WAV file too, in the microphone's place: for trying it out
without speaking (begin it with a few seconds of silence, while it
connects). Each session logs what happened (the server's messages, the
connection, the levels each second) to --log, by default
$XDG_STATE_HOME/notchd/codex-live.log; NOTCHD_LIVE_DEBUG=1 copies it to
stderr.

The rules that read these lines are core/Live.js's. Needs python-gobject,
gst-plugins-base, gst-plugins-good, gst-plugins-bad, libnice and libpulse
(pacat plays the voice), a PulseAudio server (PipeWire's pipewire-pulse) (the settings' Components page, "WebRTC and audio", installs them), and Codex logged in.
"""

import argparse
import array
import json
import math
import os
import shlex
import shutil
import subprocess
import sys
import threading
import time

RATE = 48000
# NOTCHD_LIVE_DEBUG=1: the server's messages to stderr.
DEBUG = os.environ.get("NOTCHD_LIVE_DEBUG") == "1"
# NOTCHD_LIVE_RECORD=<file.wav>: Codex's voice as it comes, decoded, to
# compare with what the speakers play (by hand; never on by default).
RECORD = os.environ.get("NOTCHD_LIVE_RECORD", "")
# How long the connection may take before it is given up.
CONNECT_TIMEOUT = 30
# How long, once live, the microphone may stay without a sound sent.
MIC_GRACE = 4
# The microphone must send this long without a gap before the connection
# opens (a microphone waking up sends late and in bursts, and the session's
# events never came through), and is given up on after MIC_TIMEOUT.
MIC_STEADY = 0.5
MIC_TIMEOUT = 10
# GStreamer elements the conversation needs, and the Arch package of each.
ELEMENTS = [
    ("webrtcbin", "gst-plugins-bad"), ("dtlssrtpenc", "gst-plugins-bad"),
    ("sctpenc", "gst-plugins-bad"), ("srtpenc", "gst-plugins-bad"),
    ("nicesrc", "libnice"), ("rtpopuspay", "gst-plugins-good"),
    ("rtpopusdepay", "gst-plugins-good"), ("opusenc", "gst-plugins-base"),
    ("opusdec", "gst-plugins-base"), ("audioconvert", "gst-plugins-base"),
    ("audioresample", "gst-plugins-base"), ("audiotestsrc", "gst-plugins-base"),
    ("volume", "gst-plugins-base"), ("appsink", "gst-plugins-base"),
    ("appsrc", "gst-plugins-base"), ("pulsesrc", "gst-plugins-good"),
    ("pitch", "gst-plugins-bad"),
]
# Codex's voice is played by pacat (PulseAudio's, pipewire-pulse serves it),
# fed as it is decoded: a player of its own, with its own buffer, and no
# clock of GStreamer's in between (see start_player).
PLAYER = "pacat"
PLAYER_PACKAGE = "libpulse"
# How much the player buffers: room for the network's unevenness, and how
# far the voice's level (the disc that swells with it) is held back to meet
# the sound.
PLAYER_LATENCY_MS = 200
# Codex's server sends its voice a little slower than it plays (about 47
# packets of 20 ms a second, measured): played as it comes, the buffer ran
# dry every few hundred ms and the voice had small cuts. As a browser's
# WebRTC does, it is stretched instead, its pitch kept (pitch's tempo):
# slower when the buffer runs low, at its pace again once there is room.
TEMPO_SLOW = 0.88
BUFFER_LOW_MS = 140
BUFFER_OK_MS = 200
# Without these it still talks, with the speakers' echo in the microphone.
ECHO_ELEMENTS = ["webrtcdsp", "webrtcechoprobe"]
# layout too: without it the echo's appsrc has caps that aren't fixed, and
# the first voice pushed into it ended the session ("Internal data stream
# error").
VOICE_CAPS = "audio/x-raw,format=S16LE,rate=%d,channels=1,layout=interleaved" % RATE
# What the speakers play, pushed in (on_voice_sample) for the echo canceller
# (webrtcdsp, on the microphone) to take out.
ECHO_BRANCH = ('appsrc name=echo is-live=true format=time do-timestamp=true caps="%s" ! '
               "webrtcechoprobe name=probe ! fakesink sync=false async=false" % VOICE_CAPS)
RTP_CAPS = "application/x-rtp,media=audio,encoding-name=OPUS,payload=111"


# ------------------------------------------------------------ pure helpers
def missing_packages(has_element):
    """The packages that bring the elements `has_element` doesn't find, in
    the order of ELEMENTS, each once."""
    out = []
    for name, package in ELEMENTS:
        if not has_element(name) and package not in out:
            out.append(package)
    return out


def own_dir(documents, home):
    """notchd's own folder, as an agent's (core/Terminal.js OWN_DIR): notchd
    in the documents folder, else in ~/Documents."""
    base = documents.strip() if documents and documents.strip() else os.path.join(home, "Documents")
    return os.path.join(base, "notchd")


def thread_params(cwd, access):
    """thread/start: a thread that goes with the session (ephemeral), never
    asking for approval, which no one would answer: what it may not do, it
    doesn't. `access` is the sandbox: read-only, or workspace-write (it may
    change files in its folder)."""
    sandbox = "workspace-write" if access == "workspace-write" else "read-only"
    return {"cwd": cwd, "ephemeral": True, "sandbox": sandbox, "approvalPolicy": "never"}


def realtime_params(thread_id, sdp, voice):
    """thread/realtime/start over WebRTC, v3: a ChatGPT login is refused v1,
    v2 and the websocket."""
    params = {"threadId": thread_id, "outputModality": "audio", "version": "v3",
              "transport": {"type": "webrtc", "sdp": sdp}}
    if voice:
        params["voice"] = voice
    return params


def translate(msg, thread_id):
    """A message from the app-server -> the lines it means for notchd. The
    voice itself never comes this way (WebRTC carries it)."""
    method = msg.get("method", "")
    p = msg.get("params") or {}
    if p.get("threadId") not in (None, thread_id):
        return []
    if method == "thread/realtime/transcript/delta":
        return [{"transcript": {"role": p.get("role", "assistant"), "delta": p.get("delta", "")}}]
    if method == "thread/realtime/transcript/done":
        return [{"transcript": {"role": p.get("role", "assistant"), "text": p.get("text", ""), "done": True}}]
    if method == "turn/started":
        return [{"working": True}]
    if method == "turn/completed":
        return [{"working": False}]
    if method == "thread/realtime/error":
        return [{"error": error_text(p.get("message", ""))}]
    if method == "thread/realtime/closed":
        return [{"status": "closed", "reason": p.get("reason", "")}]
    if method == "error":
        err = p.get("error") or {}
        return [{"warning": err.get("message", "") or "Codex reported an error"}]
    return []


def error_text(message):
    """The server's error, in one line: its JSON body's message if it has one."""
    text = str(message or "").strip()
    try:
        body = json.loads(text)
        inner = body.get("error", body) if isinstance(body, dict) else None
        if isinstance(inner, dict) and inner.get("message"):
            text = inner["message"]
    except ValueError:
        pass
    return " ".join(text.split()) or "The conversation ended with an error."


def rpc_error_text(method, error):
    """A JSON-RPC error answer -> what to say. A Codex without realtime
    doesn't know the method."""
    message = (error or {}).get("message", "") if isinstance(error, dict) else str(error)
    if (isinstance(error, dict) and error.get("code") == -32601) or "unknown variant" in message \
            or "method not found" in message.lower():
        return "This Codex has no realtime conversation (%s); update Codex." % method
    return "%s: %s" % (method, message or "failed")


def rms_db(samples):
    """The level of 16-bit samples, in dB below full scale (-100 for silence)."""
    if not samples:
        return -100.0
    total = 0
    for s in samples:
        total += s * s
    mean = total / len(samples) / (32768.0 * 32768.0)
    if mean <= 1e-10:
        return -100.0
    return max(-100.0, round(10 * math.log10(mean), 1))


def parse_command(line):
    """A line of stdin -> ("mute", bool) | ("stop", None) | None for anything
    else."""
    try:
        cmd = json.loads(line)
    except ValueError:
        return None
    if not isinstance(cmd, dict):
        return None
    if isinstance(cmd.get("mute"), bool):
        return ("mute", cmd["mute"])
    if cmd.get("stop"):
        return ("stop", None)
    return None


# --------------------------------------------------------------- the output
_out_lock = threading.Lock()


_log_file = None
_started = time.monotonic()


def log(text):
    line = "%7.2f %s\n" % (time.monotonic() - _started, text)
    with _out_lock:
        if _log_file:
            _log_file.write(line)
            _log_file.flush()
        if DEBUG:
            sys.stderr.write(line)


def emit(obj):
    if "level" not in obj and "mic" not in obj:
        log("-> %s" % json.dumps(obj, ensure_ascii=False))
    with _out_lock:
        sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
        sys.stdout.flush()


def log_path(state_home, home):
    """Where a session's log goes: $XDG_STATE_HOME/notchd/codex-live.log."""
    base = state_home if state_home else os.path.join(home, ".local", "state")
    return os.path.join(base, "notchd", "codex-live.log")


# -------------------------------------------------------------- the session
class Session:
    def __init__(self, args, GLib, Gst, GstSdp, GstWebRTC):
        self.args = args
        self.GLib, self.Gst, self.GstSdp, self.GstWebRTC = GLib, Gst, GstSdp, GstWebRTC
        self.loop = GLib.MainLoop()
        self.server = None
        self.thread_id = None
        self.pipe = None
        self.webrtc = None
        self.channel = None
        self.offered = False
        self.sent_offer = False
        self.live = False
        self.ending = False
        self.exit_code = 0
        self.next_id = 0
        self.answers = {}
        self.levels = []
        self.level_at = 0.0
        self.silent = True
        self.echo = False
        self.mic_samples = []
        self.mic_at = 0.0
        self.mic_first = 0.0
        self.mic_last = 0.0
        self.mic_ready = args.mic != "default"
        self.negotiation_wanted = False
        self.ready_parts = set()
        self.player = None
        self.echo_src = None
        # The player's buffer, as far as we can tell: what was handed over
        # since it last ran dry, less the time since.
        self.voice_start = 0.0
        self.voice_time = 0.0
        self.buffer_low = 1e9
        self.slow = False
        self.slow_count = 0
        self.record = None
        if RECORD:
            import wave
            self.record = wave.open(RECORD, "wb")
            self.record.setnchannels(1)
            self.record.setsampwidth(2)
            self.record.setframerate(RATE)
        self.mic_peak = -100.0
        self.mic_loudest = -100.0
        self.voice_peak = -100.0
        self.summary_at = 0.0

    # ------------------------------------------------------------- server
    def start_server(self):
        command = shlex.split(self.args.codex) + ["app-server"]
        try:
            self.server = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                           stderr=subprocess.DEVNULL, text=True, bufsize=1)
        except OSError as e:
            self.fail("Codex couldn't start (%s): %s" % (self.args.codex, e.strerror or e))
            return
        threading.Thread(target=self.read_server, daemon=True).start()
        self.call("initialize", {"clientInfo": {"name": "notchd", "version": "1"},
                                 "capabilities": {"experimentalApi": True}}, self.initialized)

    def read_server(self):
        for line in self.server.stdout:
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            self.GLib.idle_add(self.on_server, msg)
        self.GLib.idle_add(self.server_gone)

    def server_gone(self):
        if not self.ending:
            self.fail("Codex's server stopped.")
        return False

    def call(self, method, params, then=None):
        self.next_id += 1
        self.answers[self.next_id] = (method, then)
        self.send({"id": self.next_id, "method": method, "params": params})

    def send(self, msg):
        log(">> %s" % json.dumps(msg, ensure_ascii=False)[:300])
        try:
            self.server.stdin.write(json.dumps(msg) + "\n")
            self.server.stdin.flush()
        except (OSError, ValueError):
            pass

    def on_server(self, msg):
        method = str(msg.get("method", ""))
        if not method.startswith("mcpServer/") and method != "thread/realtime/transcript/delta":
            log("<< %s" % json.dumps(msg, ensure_ascii=False)[:400])
        if "id" in msg and "method" not in msg:
            method, then = self.answers.pop(msg["id"], (None, None))
            if "error" in msg:
                if method == "thread/realtime/stop":
                    return False
                self.fail(rpc_error_text(method, msg["error"]))
            elif then:
                then(msg.get("result") or {})
            return False
        if "id" in msg:
            # A request of the server's (an approval): no one is there to
            # answer, so no.
            self.send({"id": msg["id"], "result": {"decision": "decline"}})
            return False
        if msg.get("method") == "thread/realtime/sdp":
            self.set_answer((msg.get("params") or {}).get("sdp", ""))
            return False
        for event in translate(msg, self.thread_id):
            if "error" in event:
                self.fail(event["error"])
            elif event.get("status") == "closed":
                if not self.ending:
                    self.finish(event.get("reason") or "closed")
            else:
                emit(event)
        return False

    def initialized(self, _result):
        self.send({"method": "initialized"})
        self.call("thread/start", thread_params(self.args.cwd, self.args.access), self.thread_started)

    def thread_started(self, result):
        self.thread_id = (result.get("thread") or {}).get("id")
        if not self.thread_id:
            self.fail("Codex made no thread.")
            return
        self.start_media()

    # -------------------------------------------------------------- media
    def start_media(self):
        Gst = self.Gst
        mic = self.args.mic == "default"
        self.echo = mic and self.args.echo == "on" and all(Gst.ElementFactory.find(e) for e in ECHO_ELEMENTS)
        if mic:
            # Through pipewire-pulse: pipewiresrc holds the pipeline in
            # PAUSED until the microphone sends something.
            source = "pulsesrc client-name=notchd provide-clock=false ! audioconvert ! audioresample ! %s ! " % VOICE_CAPS
            if self.echo:
                # What it plays comes in through the echo appsrc, not in
                # step with the microphone: let it find the delay itself.
                source += "webrtcdsp name=dsp probe=probe delay-agnostic=true ! "
        elif self.args.mic == "none":
            source = "audiotestsrc is-live=true wave=silence ! %s ! " % VOICE_CAPS
        else:
            # A recording in the microphone's place, at its pace: for trying
            # it out without speaking. Give it silence first, while it connects.
            source = ("filesrc location=%s ! wavparse ! audioconvert ! audioresample ! %s ! identity sync=true ! "
                      % (shlex.quote(self.args.mic), VOICE_CAPS))
        # Codex's voice: decoded (filling in a lost packet, plc) and handed
        # over as it comes; the player plays it. An audio sink here, synced
        # to GStreamer's clock, dropped or cut 200 ms whenever the network's
        # time and the sound card's disagreed: the voice stuttered.
        voice = ("rtpopusdepay name=depay ! opusdec plc=true ! audioconvert ! pitch name=tempo ! "
                 "audioconvert ! audioresample ! %s ! appsink name=voice emit-signals=true sync=false" % VOICE_CAPS)
        if self.echo:
            voice += " " + ECHO_BRANCH
        desc = ("webrtcbin name=webrtc bundle-policy=max-bundle stun-server=stun://stun.l.google.com:19302 "
                + source + "volume name=mic ! "
                # webrtcbin holds what comes in until it is connected: held,
                # the microphone stopped, then came in late and in a burst.
                # Leaky, it runs from the start and only fresh sound goes on.
                + "queue leaky=downstream max-size-time=200000000 max-size-buffers=0 max-size-bytes=0 ! "
                + "audioconvert ! opusenc ! rtpopuspay pt=111 ! "
                + RTP_CAPS + " ! webrtc. " + voice)
        try:
            self.pipe = Gst.parse_launch(desc)
        except Exception as e:  # GLib.Error: an element is missing or won't link
            self.fail("The audio couldn't be set up: %s" % e)
            return
        if not self.echo and mic and self.args.echo == "on":
            emit({"warning": "No echo cancellation (webrtcdsp): use headphones, or Codex hears itself."})
        self.webrtc = self.pipe.get_by_name("webrtc")
        # The codec up front: without it the offer waits for the
        # microphone's first sound, and one that sends nothing never connects.
        transceiver = self.webrtc.emit("get-transceiver", 0)
        if transceiver:
            transceiver.set_property("codec-preferences", Gst.Caps.from_string(
                RTP_CAPS + ",clock-rate=48000,encoding-params=(string)2"))
        self.webrtc.connect("on-negotiation-needed", self.on_negotiation_needed)
        self.webrtc.connect("notify::ice-gathering-state", self.on_gathering)
        self.webrtc.connect("notify::ice-connection-state", self.on_ice)
        self.webrtc.connect("pad-added", self.on_pad)
        self.start_player()
        if not self.player:
            return
        self.pipe.get_by_name("voice").connect("new-sample", self.on_voice_sample)
        self.echo_src = self.pipe.get_by_name("echo")
        self.pipe.get_by_name("mic").get_static_pad("src").add_probe(
            Gst.PadProbeType.BUFFER, self.on_mic_buffer)
        log("audio: mic %s, echo cancellation %s" % (self.args.mic, "on" if self.echo else "off"))
        bus = self.pipe.get_bus()
        bus.add_signal_watch()
        bus.connect("message::error", self.on_bus_error)
        bus.connect("message::warning", self.on_bus_warning)
        self.GLib.timeout_add_seconds(1, self.log_levels)
        self.pipe.set_state(Gst.State.READY)
        # The events' channel, before the offer so the offer has it.
        self.channel = self.webrtc.emit("create-data-channel", "oai-events", None)
        if self.channel:
            self.channel.connect("on-open", lambda _c: self.GLib.idle_add(self.ready, "channel"))
            self.channel.connect("on-message-string", self.on_channel_message)
        self.pipe.set_state(Gst.State.PLAYING)
        if self.mic_ready:
            self.on_mic_ready()
        else:
            log("waiting for the microphone")
            self.GLib.timeout_add_seconds(MIC_TIMEOUT, self.mic_timeout)

    def on_mic_ready(self):
        if self.ending:
            return False
        emit({"status": "connecting"})
        self.GLib.timeout_add_seconds(CONNECT_TIMEOUT, self.connect_timeout)
        if self.negotiation_wanted:
            self.make_offer()
        return False

    def mic_timeout(self):
        if not self.mic_ready:
            self.fail("The microphone sends nothing: check the default input (wpctl status, pavucontrol).")
        return False

    def on_negotiation_needed(self, _webrtc):
        self.negotiation_wanted = True
        if self.mic_ready:
            self.make_offer()

    def make_offer(self):
        if self.offered or self.ending:
            return
        self.offered = True
        promise = self.Gst.Promise.new_with_change_func(self.on_offer, None, None)
        self.webrtc.emit("create-offer", None, promise)

    def on_offer(self, promise, *_):
        # Keep the reply: the offer is borrowed from it, and freed with it.
        reply = promise.get_reply()
        if reply is None or reply.has_field("error"):
            self.GLib.idle_add(self.fail, "WebRTC made no offer.")
            return
        self.offer = reply.get_value("offer")
        self.webrtc.emit("set-local-description", self.offer, self.Gst.Promise.new())

    def on_gathering(self, _webrtc, _pspec):
        # The server answers once, without trickle: the offer goes with
        # every candidate.
        if self.webrtc.get_property("ice-gathering-state") != self.GstWebRTC.WebRTCICEGatheringState.COMPLETE:
            return
        if self.sent_offer:
            return
        self.sent_offer = True
        sdp = self.webrtc.get_property("local-description").sdp.as_text()
        self.GLib.idle_add(self.send_offer, sdp)

    def send_offer(self, sdp):
        self.call("thread/realtime/start", realtime_params(self.thread_id, sdp, self.args.voice))
        return False

    def set_answer(self, text):
        ok, sdp = self.GstSdp.SDPMessage.new_from_text(text)
        if ok != self.GstSdp.SDPResult.OK:
            self.fail("The server's answer couldn't be read.")
            return
        answer = self.GstWebRTC.WebRTCSessionDescription.new(self.GstWebRTC.WebRTCSDPType.ANSWER, sdp)
        self.webrtc.emit("set-remote-description", answer, self.Gst.Promise.new())

    def on_ice(self, _webrtc, _pspec):
        state = self.webrtc.get_property("ice-connection-state")
        log("ice: %s" % state.value_nick)
        if state == self.GstWebRTC.WebRTCICEConnectionState.FAILED:
            self.GLib.idle_add(self.fail, "The connection to the voice server failed.")

    def on_pad(self, _webrtc, pad):
        if pad.direction != self.Gst.PadDirection.SRC:
            return
        sink = self.pipe.get_by_name("depay").get_static_pad("sink")
        if not sink.is_linked():
            log("voice: %s" % pad.link(sink).value_nick)
            self.GLib.idle_add(self.ready, "voice")

    def on_channel_message(self, _channel, text):
        # The session's events, by type, for the log: when it started, what
        # it was told. The transcript comes from the server too.
        try:
            kind = json.loads(text).get("type", "?")
        except (ValueError, AttributeError):
            kind = "?"
        if not kind.endswith(".delta") and not kind.endswith(".added"):
            self.GLib.idle_add(log, "event: %s" % kind)

    def ready(self, part):
        """Live once the events' channel is open and Codex's voice has come:
        before its voice, what you say is lost (it isn't listening yet)."""
        log("ready: %s" % part)
        self.ready_parts.add(part)
        if {"channel", "voice"} <= self.ready_parts:
            self.on_live()
        return False

    def on_live(self):
        if not self.live and not self.ending:
            self.live = True
            emit({"status": "live"})
            if self.args.mic == "default":
                self.GLib.timeout_add_seconds(MIC_GRACE, self.check_mic)
        return False

    def check_mic(self):
        # Talking to it is the point: a microphone that sends nothing is
        # worth saying, without ending the session (it may come back).
        if self.ending:
            return False
        if self.mic_at == 0.0:
            emit({"warning": "The microphone sends nothing: check the default input (wpctl status, pavucontrol)."})
        elif self.mic_loudest <= -90:
            emit({"warning": "The microphone sends only silence: is it muted?"})
        return False

    def connect_timeout(self):
        if not self.live and not self.ending:
            self.fail("The voice server didn't connect in %d s." % CONNECT_TIMEOUT)
        return False

    def on_bus_warning(self, _bus, message):
        err, debug = message.parse_warning()
        log("gst warning: %s (%s)" % (err.message, (debug or "").splitlines()[0] if debug else ""))

    def log_levels(self):
        if self.ending:
            return False
        log("levels: mic peak %.0f dB, voice peak %.0f dB, voice buffered %s, slowed %d times"
            % (self.mic_peak, self.voice_peak,
               "-" if self.buffer_low > 1e8 else "%.0f ms at least" % (self.buffer_low * 1000), self.slow_count))
        self.mic_peak = self.voice_peak = -100.0
        self.buffer_low = 1e9
        self.slow_count = 0
        return True

    def samples_of(self, info):
        buf = info.get_buffer()
        ok, mapped = buf.map(self.Gst.MapFlags.READ)
        if not ok:
            return None
        data = bytes(mapped.data)
        buf.unmap(mapped)
        samples = array.array("h")
        samples.frombytes(data[: len(data) - len(data) % 2])
        return samples

    def on_mic_buffer(self, _pad, info):
        samples = self.samples_of(info)
        if samples is not None and not self.mic_ready:
            # Steady: buffers without a gap, for MIC_STEADY.
            now = time.monotonic()
            if self.mic_first == 0.0 or now - self.mic_last > 0.1:
                self.mic_first = now
            self.mic_last = now
            if now - self.mic_first >= MIC_STEADY:
                self.mic_ready = True
                self.GLib.idle_add(log, "microphone ready")
                self.GLib.idle_add(self.on_mic_ready)
        if samples is not None:
            self.mic_samples.extend(samples)
            now = time.monotonic()
            if now - self.mic_at >= 0.1:
                db = rms_db(self.mic_samples)
                self.mic_samples = []
                self.mic_at = now
                self.mic_peak = max(self.mic_peak, db)
                self.mic_loudest = max(self.mic_loudest, db)
                emit({"mic": db})
        return self.Gst.PadProbeReturn.OK

    def on_bus_error(self, _bus, message):
        err, _debug = message.parse_error()
        self.GLib.idle_add(self.fail, "Audio: %s" % err.message)

    # ------------------------------------------------------------- player
    def start_player(self):
        """pacat, fed the voice as it is decoded: it plays at the sound
        card's pace from a buffer of its own, which takes up the network's
        unevenness."""
        try:
            self.player = subprocess.Popen(
                [PLAYER, "--playback", "--raw", "--format=s16le", "--rate=%d" % RATE, "--channels=1",
                 "--latency-msec=%d" % PLAYER_LATENCY_MS, "--client-name=notchd", "--stream-name=Codex"],
                stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, bufsize=0)
        except OSError as e:
            self.player = None
            self.fail("Codex's voice can't be played (%s: %s)." % (PLAYER, e.strerror or e), [PLAYER_PACKAGE])

    def on_voice_sample(self, sink):
        sample = sink.emit("pull-sample")
        if sample is None:
            return self.Gst.FlowReturn.OK
        buf = sample.get_buffer()
        ok, mapped = buf.map(self.Gst.MapFlags.READ)
        if not ok:
            return self.Gst.FlowReturn.OK
        data = bytes(mapped.data)
        buf.unmap(mapped)
        if self.player and self.player.poll() is None:
            try:
                self.player.stdin.write(data)
            except (BrokenPipeError, OSError):
                self.GLib.idle_add(self.fail, "Codex's voice stopped playing (%s ended)." % PLAYER)
        if self.echo_src:
            self.echo_src.emit("push-buffer", self.Gst.Buffer.new_wrapped(data))
        if self.record:
            self.record.writeframes(data)
        self.keep_buffer(len(data) / 2.0 / RATE)
        samples = array.array("h")
        samples.frombytes(data[: len(data) - len(data) % 2])
        self.levels.extend(samples)
        now = time.monotonic()
        if now - self.level_at >= 0.04:
            db = rms_db(self.levels)
            self.levels = []
            self.level_at = now
            self.voice_peak = max(self.voice_peak, db)
            if db > -60 or not self.silent:
                self.silent = db <= -60
                # When the player plays it, not when it is handed over.
                self.GLib.timeout_add(PLAYER_LATENCY_MS, self.emit_level, db if db > -60 else -100)
        return self.Gst.FlowReturn.OK

    def keep_buffer(self, seconds):
        """Stretches the voice while the player's buffer runs low (see
        TEMPO_SLOW). The player starts once it holds PLAYER_LATENCY_MS, so
        what it holds is what was handed over since it last ran dry, less
        the time since, plus that."""
        now = time.monotonic()
        latency = PLAYER_LATENCY_MS / 1000.0
        held = self.voice_time - (now - self.voice_start) + latency
        if self.voice_start == 0.0 or held < 0:
            # Ran dry (a pause, the start): it fills again from here.
            self.voice_start, self.voice_time, held = now, 0.0, latency
        self.voice_time += seconds
        held += seconds
        self.buffer_low = min(self.buffer_low, held)
        if not self.slow and held * 1000 < BUFFER_LOW_MS:
            self.set_tempo(True)
        elif self.slow and held * 1000 > BUFFER_OK_MS:
            self.set_tempo(False)

    def set_tempo(self, slow):
        self.slow = slow
        if slow:
            self.slow_count += 1
        tempo = self.pipe.get_by_name("tempo")
        if tempo:
            tempo.set_property("tempo", TEMPO_SLOW if slow else 1.0)

    def emit_level(self, db):
        if not self.ending:
            emit({"level": db})
        return False

    # ---------------------------------------------------------- commands
    def read_commands(self):
        for line in sys.stdin:
            cmd = parse_command(line)
            if cmd:
                self.GLib.idle_add(self.on_command, cmd)
        self.GLib.idle_add(self.on_command, ("stop", None))

    def on_command(self, cmd):
        kind, value = cmd
        if kind == "mute" and self.pipe:
            self.pipe.get_by_name("mic").set_property("mute", value)
            emit({"muted": value})
        elif kind == "stop":
            self.finish("requested")
        return False

    # ------------------------------------------------------------- ending
    def fail(self, message, packages=None):
        if self.ending:
            return False
        event = {"error": message}
        if packages:
            event["packages"] = packages
        emit(event)
        self.exit_code = 2 if packages else 1
        self.finish(None)
        return False

    def finish(self, reason):
        if self.ending:
            return False
        self.ending = True
        if self.thread_id and self.server and self.server.poll() is None:
            self.call("thread/realtime/stop", {"threadId": self.thread_id})
        self.GLib.timeout_add(600, self.quit, reason)
        return False

    def quit(self, reason):
        if self.pipe:
            self.pipe.set_state(self.Gst.State.NULL)
        if self.player and self.player.poll() is None:
            try:
                self.player.stdin.close()
            except OSError:
                pass
            try:
                self.player.wait(1)
            except subprocess.TimeoutExpired:
                self.player.kill()
        if self.server and self.server.poll() is None:
            try:
                self.server.stdin.close()
            except OSError:
                pass
            try:
                self.server.wait(2)
            except subprocess.TimeoutExpired:
                self.server.terminate()
        if self.record:
            self.record.close()
        emit({"status": "closed", "reason": reason or "error"})
        self.loop.quit()
        return False

    def on_signal(self, signum):
        try:
            from gi.repository import GLibUnix
            add = GLibUnix.signal_add
        except ImportError:
            add = self.GLib.unix_signal_add
        add(self.GLib.PRIORITY_DEFAULT, signum, lambda: self.finish("requested") or True)

    def run(self):
        emit({"status": "starting"})
        for signum in (2, 15):
            self.on_signal(signum)
        threading.Thread(target=self.read_commands, daemon=True).start()
        self.start_server()
        self.loop.run()
        return self.exit_code


def load_gstreamer():
    """(GLib, Gst, GstSdp, GstWebRTC), or the packages to install."""
    try:
        import gi
        gi.require_version("Gst", "1.0")
        gi.require_version("GstSdp", "1.0")
        gi.require_version("GstWebRTC", "1.0")
        from gi.repository import GLib, Gst, GstSdp, GstWebRTC
    except (ImportError, ValueError):
        return None, ["python-gobject", "gst-plugins-bad-libs"]
    Gst.init(None)
    missing = missing_packages(lambda name: Gst.ElementFactory.find(name) is not None)
    if not shutil.which(PLAYER):
        missing.append(PLAYER_PACKAGE)
    if missing:
        return None, missing
    return (GLib, Gst, GstSdp, GstWebRTC), []


def resolve_cwd(cwd):
    if cwd:
        return os.path.expanduser(cwd)
    home = os.path.expanduser("~")
    try:
        documents = subprocess.run(["xdg-user-dir", "DOCUMENTS"], capture_output=True, text=True,
                                   timeout=5).stdout
    except (OSError, subprocess.TimeoutExpired):
        documents = ""
    # xdg-user-dir answers $HOME when DOCUMENTS isn't set.
    if documents.strip() == home:
        documents = ""
    path = own_dir(documents, home)
    os.makedirs(path, exist_ok=True)
    return path


def main(argv=None):
    parser = argparse.ArgumentParser(description="A spoken conversation with Codex, for notchd.")
    parser.add_argument("--codex", default="codex")
    parser.add_argument("--cwd", default="")
    parser.add_argument("--voice", default="")
    parser.add_argument("--access", default="read-only", choices=["read-only", "workspace-write"])
    # "default": the default microphone; "none": silence; else a WAV file.
    parser.add_argument("--mic", default="default")
    # Echo cancellation (webrtcdsp): off with headphones.
    parser.add_argument("--echo", default="on", choices=["on", "off"])
    parser.add_argument("--log", default=log_path(os.environ.get("XDG_STATE_HOME", ""), os.path.expanduser("~")))
    args = parser.parse_args(argv)
    global _log_file
    try:
        os.makedirs(os.path.dirname(args.log), exist_ok=True)
        _log_file = open(args.log, "w", encoding="utf-8")
    except OSError:
        _log_file = None
    log("codex-live: %s" % " ".join(sys.argv[1:]))
    modules, packages = load_gstreamer()
    if not modules:
        emit({"error": "Codex Live needs packages that aren't installed.", "packages": packages,
              "components": ["codex-live-audio"]})
        return 2
    args.cwd = resolve_cwd(args.cwd)
    if not os.path.isdir(args.cwd):
        emit({"error": "No such folder: %s" % args.cwd})
        return 1
    return Session(args, *modules).run()


if __name__ == "__main__":
    sys.exit(main())
