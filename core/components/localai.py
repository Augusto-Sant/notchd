"""LocalAI (https://localai.io), the engine notchd's voices run on: where
notchd keeps it, how to reach it and how to start it. Shared by the
components' installer (install.py) and the Voice bridge (../live/local-live.py).
Standard library only.

notchd's own copy lives in $XDG_DATA_HOME/notchd/localai: bin/local-ai (the
official release binary, no Docker, no root), models/, backends/ and hf/ (the
weights some backends fetch on first use; HF_HOME points there). Nothing is
written anywhere else.

It is started on demand, bound to localhost, by whoever needs it (the
installer to install a voice, the bridge for a conversation) and stopped by
the same when done: an idle LocalAI costs nothing because there is none. One
that already answers at the address (the user's own, or another process of
notchd) is used as it is and never stopped.

Test hooks: $NOTCHD_LOCALAI_URL (the address), $NOTCHD_LOCALAI_BIN (the
program: a fake).
"""

import json
import os
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_URL = "http://127.0.0.1:18080"
# What the Voice setting is when empty is the address of notchd's own copy.
START_TIMEOUT = 90.0


def data_dir():
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "notchd", "localai")


def models_dir():
    return os.path.join(data_dir(), "models")


def backends_dir():
    return os.path.join(data_dir(), "backends")


def binary():
    return os.environ.get("NOTCHD_LOCALAI_BIN") or os.path.join(data_dir(), "bin", "local-ai")


def log_path():
    base = os.environ.get("XDG_STATE_HOME") or os.path.join(os.path.expanduser("~"), ".local", "state")
    return os.path.join(base, "notchd", "localai.log")


def base_url(url=""):
    u = (url or os.environ.get("NOTCHD_LOCALAI_URL") or DEFAULT_URL).strip().rstrip("/")
    return u if "://" in u else "http://" + u


def local_address(url):
    """host:port to listen on when `url` is an address on this machine that
    notchd may start LocalAI at (http, localhost, 127.x or ::1); None for any
    other (a server of someone else's)."""
    p = urllib.parse.urlparse(base_url(url))
    host = p.hostname or ""
    if p.scheme != "http" or host not in ("localhost", "127.0.0.1", "::1"):
        return None
    return "%s:%d" % ("[::1]" if host == "::1" else "127.0.0.1" if host == "localhost" else host, p.port or 80)


def free_port():
    """A port nobody listens on (LocalAI's profiling endpoint wants one of
    its own, 6060 by default, which other programs use)."""
    import socket
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def request(method, url, body=None, timeout=30):
    """(status, bytes) of one HTTP request; (0, b"") when nothing answers.
    `body` is sent as JSON."""
    data = None
    headers = {"User-Agent": "notchd"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    try:
        with urllib.request.urlopen(urllib.request.Request(url, data=data, headers=headers, method=method),
                                    timeout=timeout) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        try:
            return e.code, e.read()
        except OSError:
            return e.code, b""
    except (urllib.error.URLError, OSError, ValueError):
        return 0, b""


def reachable(url, timeout=2):
    return request("GET", base_url(url) + "/readyz", timeout=timeout)[0] == 200


def error_message(status, body):
    """A LocalAI error response -> its words."""
    try:
        e = json.loads(body.decode("utf-8", "replace")).get("error")
        if isinstance(e, dict):
            e = e.get("message")
        if e:
            return str(e)
    except (ValueError, AttributeError):
        pass
    return "LocalAI answered %s." % status if status else "LocalAI does not answer."


class Server:
    """notchd's LocalAI as a child, when there is none at the address."""

    def __init__(self, url="", log=None):
        self.url = base_url(url)
        self.proc = None
        self.log = log

    def ensure(self, should_stop=lambda: False):
        """Returns "" when LocalAI answers at the address (started here or
        already running), else why not."""
        if reachable(self.url):
            return ""
        address = local_address(self.url)
        exe = binary()
        if address is None:
            return "LocalAI does not answer at %s." % self.url
        if not os.access(exe, os.X_OK):
            return "LocalAI is not installed."
        root = data_dir()
        for d in (models_dir(), backends_dir(), os.path.join(root, "hf"), os.path.join(root, "data"),
                  os.path.join(root, "tmp"), os.path.dirname(log_path())):
            os.makedirs(d, exist_ok=True)
        env = dict(os.environ, HF_HOME=os.path.join(root, "hf"), HUGGINGFACE_HUB_CACHE=os.path.join(root, "hf"))
        try:
            out = open(log_path(), "ab")
        except OSError:
            out = subprocess.DEVNULL
        cmd = [exe, "run", "--address", address, "--models-path", models_dir(), "--backends-path", backends_dir(),
               "--data-path", os.path.join(root, "data"), "--localai-config-dir", os.path.join(root, "config"),
               "--generated-content-path", os.path.join(root, "tmp", "generated"),
               "--upload-path", os.path.join(root, "tmp", "upload"),
               "--disable-web-ui", "--pprof-address", "127.0.0.1:%d" % free_port()]
        try:
            self.proc = subprocess.Popen(cmd, cwd=root, env=env, stdin=subprocess.DEVNULL, stdout=out, stderr=out,
                                         start_new_session=True)
        except OSError as e:
            return "Could not start LocalAI: %s." % e
        end = time.time() + START_TIMEOUT
        while time.time() < end:
            if should_stop():
                self.stop()
                return "Stopped."
            if self.proc.poll() is not None:
                self.proc = None
                return "LocalAI stopped when it started (see %s)." % log_path()
            if reachable(self.url, timeout=1):
                return ""
            time.sleep(0.2)
        self.stop()
        return "LocalAI did not start in %d seconds (see %s)." % (START_TIMEOUT, log_path())

    @property
    def owned(self):
        return self.proc is not None

    def stop(self):
        p, self.proc = self.proc, None
        if p is None or p.poll() is not None:
            return
        for sig, wait in ((signal.SIGTERM, 8), (signal.SIGKILL, 3)):
            try:
                os.killpg(p.pid, sig)
            except OSError:
                pass
            try:
                p.wait(timeout=wait)
                return
            except subprocess.TimeoutExpired:
                continue


if __name__ == "__main__":
    print(__doc__, file=sys.stderr)
    sys.exit(2)
