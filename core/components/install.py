#!/usr/bin/env python3
"""Installs, removes and reports notchd's optional components: what the
voice modules need beyond the app itself (LocalAI and its voices, Arch
packages). The settings window runs this; so can you.

    install.py status                  JSON: every component's state
    install.py install <id>            starts the installation, detached, and returns
    install.py run <id>                installs in this process (what `install` starts)
    install.py cancel <id>             stops one that is running, and cleans up
    install.py remove <id>             deletes what it installed (never a system package)

The components are in components.json next to this file (override:
$NOTCHD_COMPONENTS_FILE), of three kinds:

  files   downloads into $XDG_DATA_HOME/notchd/<dir>: into <name>.part, renamed
          when its size is what the server said (`exec` makes it runnable). A
          failed download keeps the .part and the next try resumes it (Range);
          a cancelled one is deleted. `remove_dir` makes Remove delete the
          whole <dir> (LocalAI's, with the voices it holds).
  localai a model of LocalAI, installed through its HTTP API (localai.py starts
          it for the time it takes): its backend, then the model (both with
          the byte progress LocalAI reports), then a first request that makes
          the backend fetch its weights. `installed.json`-like marker last: no
          marker, not installed. Remove deletes what it declares in `paths`.
  system  packages that need root (pacman). With pkexec when a polkit
          authentication agent runs, else in a terminal that runs sudo. An
          `aur` one is built by yay or paru in a terminal. Never removed from
          here: the system shares them.

A running installation writes $XDG_STATE_HOME/notchd/components/<id>.json
(atomically): {"phase", "detail", "done", "total", "error", "pid", "updated"},
phase one of queued, downloading, installing, done, failed, cancelled.
Closing whatever started it does not stop it.

Standard library only. Test hooks: $NOTCHD_COMPONENTS_FILE, $NOTCHD_POLKIT_AGENT
(1 or 0), $NOTCHD_TERMINAL (a terminal's command line); localai.py's
$NOTCHD_LOCALAI_URL and $NOTCHD_LOCALAI_BIN.
"""

import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import localai  # noqa: E402  (next to this file)

ACTIVE = ("queued", "downloading", "installing")
CHUNK = 256 * 1024
USER_AGENT = "notchd-components"


class Failed(Exception):
    pass


class Cancelled(Exception):
    pass


# ------------------------------------------------------------------ paths

def data_root():
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "notchd")


def state_dir():
    base = os.environ.get("XDG_STATE_HOME") or os.path.join(os.path.expanduser("~"), ".local", "state")
    return os.path.join(base, "notchd", "components")


def state_path(cid):
    return os.path.join(state_dir(), cid + ".json")


def catalog():
    path = os.environ.get("NOTCHD_COMPONENTS_FILE") or os.path.join(HERE, "components.json")
    with open(path, encoding="utf-8") as f:
        return json.load(f)["components"]


def find(cid):
    for c in catalog():
        if c["id"] == cid:
            return c
    return None


def component_dir(c):
    return os.path.join(data_root(), c["dir"])


def marker_path(c):
    """Written last by a LocalAI model's installation."""
    return os.path.join(localai.data_dir(), "markers", c["id"] + ".json")


def model_name(c):
    return c["model"]["name"]


def model_paths(c):
    """What a LocalAI model takes (and Remove deletes), under LocalAI's folder."""
    return [os.path.join(localai.data_dir(), p) for p in c.get("paths", [])]


def holds(c, path):
    """Whether `c` is installed (or being installed) and declares `path`."""
    return c["kind"] == "localai" and path in c.get("paths", []) and (
        is_installed(c) or is_active(read_state(c["id"])))


def shared_with(c, path):
    """The other components that hold `path` too: a backend (cpu-pocket-tts)
    and the weights cache (hf) serve every language of an engine, so what one
    removes the others still need."""
    return [o for o in catalog() if o["id"] != c["id"] and holds(o, path)]


def owns_shared(c, path):
    """Whether `path`, which several components hold, counts toward `c`: the
    first holder in the registry's order, so a size is never counted twice."""
    for o in catalog():
        if o["id"] == c["id"] or holds(o, path):
            return o["id"] == c["id"]
    return True


def hf_root():
    return os.path.join(localai.data_dir(), "hf")


def hf_repos():
    """The repositories of the Hugging Face cache (hf/models--org--name...)."""
    try:
        return [os.path.join(hf_root(), d) for d in sorted(os.listdir(hf_root()))
                if "--" in d and os.path.isdir(os.path.join(hf_root(), d, "snapshots"))]
    except OSError:
        return []


def hf_snapshots():
    """The folders of the cache's revisions."""
    out = []
    for repo in hf_repos():
        snaps = os.path.join(repo, "snapshots")
        out += [os.path.join(snaps, r) for r in sorted(os.listdir(snaps))]
    return out


def hf_language_dirs(language):
    return [os.path.join(s, "languages", language) for s in hf_snapshots()
            if os.path.isdir(os.path.join(s, "languages", language))]


def hf_chain(link):
    """Every path a link goes through to its file, itself first (a revision's
    file points at the repository's blob, which may point at the cache's
    shared store, hf/blobs/<xx>/<hash>)."""
    out = []
    seen = set()
    while os.path.islink(link) and link not in seen:
        seen.add(link)
        out.append(link)
        link = os.path.normpath(os.path.join(os.path.dirname(link), os.readlink(link)))
    out.append(link)
    return out


def hf_forget(language):
    """Removes one language's weights from the shared cache: its folder in
    every revision, then the blobs no revision of any repository reaches
    any more, with their lock and refs files."""
    for d in hf_language_dirs(language):
        shutil.rmtree(d, ignore_errors=True)
    used = set()
    for snap in hf_snapshots():
        for base, _dirs, names in os.walk(snap):
            for n in names:
                used.update(hf_chain(os.path.join(base, n)))
    stores = [os.path.join(r, "blobs") for r in hf_repos()] + [os.path.join(hf_root(), "blobs")]
    for store in stores:
        for base, _dirs, names in os.walk(store):
            for n in names:
                q = os.path.join(base, n)
                if q in used or (n.endswith((".lock", ".refs")) and os.path.join(base, n.rsplit(".", 1)[0]) in used):
                    continue
                try:
                    os.unlink(q)
                except OSError:
                    pass
        for base, dirs, _names in os.walk(store, topdown=False):
            for d in dirs:
                try:
                    os.rmdir(os.path.join(base, d))  # only if empty
                except OSError:
                    pass


def model_yaml(c):
    return os.path.join(localai.models_dir(), model_name(c) + ".yaml")


def gallery_note(c):
    """The hidden file LocalAI keeps next to a model it installed from its gallery."""
    return os.path.join(localai.models_dir(), "._gallery_" + model_name(c) + ".yaml")


def file_paths(c):
    return [os.path.join(component_dir(c), f["name"]) for f in c.get("files", [])]


# ------------------------------------------------------------------ state

def read_state(cid):
    try:
        with open(state_path(cid), encoding="utf-8") as f:
            s = json.load(f)
        return s if isinstance(s, dict) else {}
    except (OSError, ValueError):
        return {}


def write_state(cid, **fields):
    os.makedirs(state_dir(), exist_ok=True)
    fields["updated"] = time.time()
    path = state_path(cid)
    tmp = "%s.%d.tmp" % (path, os.getpid())
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(fields, f)
    os.replace(tmp, path)


def clear_state(cid):
    try:
        os.unlink(state_path(cid))
    except OSError:
        pass


def pid_alive(pid):
    if not isinstance(pid, int) or pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    try:  # a zombie is dead
        with open("/proc/%d/stat" % pid) as f:
            return f.read().rsplit(")", 1)[1].split()[0] != "Z"
    except (OSError, IndexError):
        return True


def is_active(state):
    """A worker is running: its phase says so and its process is there (one
    just queued has none yet: it gets a few seconds)."""
    if state.get("phase") not in ACTIVE:
        return False
    pid = state.get("pid")
    if pid:
        return pid_alive(pid)
    return time.time() - float(state.get("updated") or 0) < 15


# ------------------------------------------------------------ what exists

def pacman_names():
    """The installed packages' names, or None without pacman."""
    if not shutil.which("pacman"):
        return None
    try:
        out = subprocess.run(["pacman", "-Qq"], capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return set(out.stdout.split())


def missing_packages(c, names=None):
    names = pacman_names() if names is None else names
    if names is None:  # not Arch: the programs on PATH are the best evidence
        bins = c.get("bins") or []
        return [] if bins and all(shutil.which(b) for b in bins) else list(c["packages"])
    return [p for p in c["packages"] if p not in names]


def is_installed(c, names=None):
    kind = c["kind"]
    if kind == "files":
        return all(os.path.isfile(p) and os.path.getsize(p) > 0 and (not f.get("exec") or os.access(p, os.X_OK))
                   for f, p in zip(c["files"], file_paths(c)))
    if kind == "localai":
        return (os.path.isfile(marker_path(c)) and os.path.isfile(model_yaml(c))
                and all(os.path.exists(p) for p in model_paths(c)))
    return not missing_packages(c, names)


def tree_size(path, follow=False):
    """Bytes under `path`; a link counts for nothing, unless `follow` (a
    cache's snapshots point at its blobs, which are counted where they are)."""
    stat = os.stat if follow else os.lstat

    def size(q):
        try:
            return 0 if not follow and os.path.islink(q) else stat(q).st_size
        except OSError:
            return 0
    if os.path.isfile(path):
        return size(path)
    total = 0
    for base, _dirs, names in os.walk(path):
        for n in names:
            total += size(os.path.join(base, n))
    return total


def language_size(language):
    """What one language takes of the weights cache: its files across the
    revisions, a blob two revisions point at counted once."""
    seen = {}
    for d in hf_language_dirs(language):
        for base, _dirs, names in os.walk(d):
            for n in names:
                q = os.path.realpath(os.path.join(base, n))
                try:
                    seen[q] = os.stat(q).st_size
                except OSError:
                    pass
    return sum(seen.values())


def own_size(c):
    """What a LocalAI component takes on disk, without what others share: a
    path several installed components hold counts for the first of them, and
    the weights cache counts a language to its own component."""
    total = tree_size(model_yaml(c))
    for rel, p in zip(c.get("paths", []), model_paths(c)):
        if c.get("hf_language") and rel == "hf":
            mine = language_size(c["hf_language"])
            total += mine
            if owns_shared(c, rel):  # what no language owns: tokenizers, voices
                others = sum(language_size(o["hf_language"]) for o in catalog()
                             if o.get("hf_language") and o["id"] != c["id"])
                total += max(0, tree_size(p) - others - mine)
        elif not shared_with(c, rel) or owns_shared(c, rel):
            total += tree_size(p)
    return total


def disk_size(c):
    if c["kind"] == "system":
        return 0
    if c["kind"] == "localai":
        return own_size(c)
    total = 0
    for p in file_paths(c):
        for q in (p, p + ".part"):
            try:
                total += os.path.getsize(q)
            except OSError:
                pass
    return total


def percent_of(state):
    total, done = state.get("total"), state.get("done")
    if isinstance(total, (int, float)) and total > 0 and isinstance(done, (int, float)):
        return max(0, min(100, int(done * 100 / total)))
    return None


def entry(c, names=None):
    """One component's state for `status`."""
    st = read_state(c["id"])
    installed = is_installed(c, names)
    active = is_active(st)
    phase = st.get("phase", "")
    error = st.get("error", "")
    if phase in ACTIVE and not active:
        phase, error = "failed", "The installation stopped unexpectedly."
    if installed and not active:
        phase, error = "", ""
    if phase in ("done", "cancelled") and not active:
        phase = ""
    out = {
        "id": c["id"], "kind": c["kind"], "installed": installed, "active": active,
        "phase": phase, "detail": st.get("detail", "") if active else "",
        "done": st.get("done") if active else None, "total": st.get("total") if active else None,
        "percent": percent_of(st) if active else None,
        "error": error if phase == "failed" else "",
        "removable": c["kind"] != "system", "disk": disk_size(c),
    }
    if c["kind"] == "system":
        out["missing"] = missing_packages(c, names)
    return out


def status():
    names = pacman_names() if any(c["kind"] == "system" for c in catalog()) else None
    return {c["id"]: entry(c, names) for c in catalog()}


# --------------------------------------------------------------- progress

class Progress:
    """The state file of a running installation, written at most a few times
    a second."""

    def __init__(self, cid):
        self.cid = cid
        self.phase = "queued"
        self.detail = ""
        self.done = None
        self.total = None
        self.last = 0.0
        self.lock = threading.Lock()

    def set(self, phase=None, detail=None, done=None, total=None, force=False):
        with self.lock:
            if phase is not None:
                force = force or phase != self.phase
                self.phase = phase
            if detail is not None:
                self.detail = detail
            if done is not None:
                self.done = done
            if total is not None:
                self.total = total
            now = time.time()
            if not force and now - self.last < 0.25:
                return
            self.last = now
            write_state(self.cid, phase=self.phase, detail=self.detail, done=self.done, total=self.total,
                        error="", pid=os.getpid())

    def fail(self, message):
        with self.lock:
            write_state(self.cid, phase="failed", detail="", done=None, total=None, error=message, pid=os.getpid())

    def finish(self, phase):
        with self.lock:
            write_state(self.cid, phase=phase, detail="", done=None, total=None, error="", pid=os.getpid())


CANCEL = threading.Event()
CHILD = {"proc": None}


def on_term(_signum, _frame):
    CANCEL.set()
    kill_child()


def kill_child():
    p = CHILD["proc"]
    if p and p.poll() is None:
        try:
            os.killpg(p.pid, signal.SIGTERM)
        except OSError:
            pass


def check_cancel():
    if CANCEL.is_set():
        raise Cancelled()


# -------------------------------------------------------------- downloads

def download(url, dest, progress, before=0, known_total=0):
    """url -> dest, through dest.part (resumed when there is one). Returns
    the bytes of the file."""
    part = dest + ".part"
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    for attempt in (0, 1):
        have = os.path.getsize(part) if os.path.exists(part) else 0
        headers = {"User-Agent": USER_AGENT}
        if have:
            headers["Range"] = "bytes=%d-" % have
        try:
            resp = urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=30)
        except urllib.error.HTTPError as e:
            if e.code == 416 and attempt == 0:  # what we kept is no prefix of the file
                os.unlink(part)
                continue
            raise Failed("The server answered %d %s." % (e.code, e.reason))
        except (urllib.error.URLError, OSError) as e:
            raise Failed("Could not download: %s." % getattr(e, "reason", e))
        break
    with resp:
        length = int(resp.headers.get("Content-Length") or 0)
        if resp.status == 206:
            m = re.match(r"bytes (\d+)-\d+/(\d+)", resp.headers.get("Content-Range", ""))
            total = int(m.group(2)) if m else have + length
            mode = "ab"
        else:
            have, total, mode = 0, length, "wb"
        done = have
        progress.set("downloading", done=before + done, total=before + (total or known_total), force=True)
        try:
            with open(part, mode) as out:
                while True:
                    check_cancel()
                    chunk = resp.read(CHUNK)
                    if not chunk:
                        break
                    out.write(chunk)
                    done += len(chunk)
                    progress.set("downloading", done=before + done, total=before + (total or known_total))
        except (OSError, urllib.error.URLError) as e:
            raise Failed("The download was interrupted: %s." % e)
    if (total and done != total) or done == 0:
        if done > total > 0:
            os.unlink(part)
        raise Failed("The download is incomplete (%d of %d bytes); trying again continues it." % (done, total))
    os.replace(part, dest)
    return done


def install_files(c, progress):
    done_bytes = 0
    for f in c["files"]:
        dest = os.path.join(component_dir(c), f["name"])
        if os.path.isfile(dest) and os.path.getsize(dest) > 0:
            done_bytes += os.path.getsize(dest)
            continue
        done_bytes += download(f["url"], dest, progress, done_bytes, int(c.get("size") or 0))
    for f in c["files"]:
        if f.get("exec"):
            os.chmod(os.path.join(component_dir(c), f["name"]), 0o755)
    progress.set("installing", detail="Checking", force=True)
    if not is_installed(c):
        raise Failed("The files are not all there after the download.")


# --------------------------------------------------------------- programs

def run(cmd, progress, env=None):
    """Runs cmd, its output's last line as the detail; Failed with the last
    lines when it fails."""
    check_cancel()
    try:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                                text=True, errors="replace", env=env, start_new_session=True)
    except OSError as e:
        raise Failed("Could not run %s: %s." % (cmd[0], e))
    CHILD["proc"] = proc
    tail = []
    try:
        for line in proc.stdout:
            line = line.strip()
            if line:
                tail = (tail + [line])[-12:]
                progress.set(detail=line[:140])
        code = proc.wait()
    finally:
        CHILD["proc"] = None
    check_cancel()
    if code != 0:
        raise Failed("%s failed (%d): %s" % (os.path.basename(cmd[0]), code, " | ".join(tail[-4:])))
    return tail


# ----------------------------------------------------------------- LocalAI

JOB_POLL = 0.5


def job_failure(job):
    err = job.get("error")
    if isinstance(err, dict):
        err = err.get("message")
    return str(err) if err else ""


def gallery_job(server_url, kind, request, progress, label):
    """A LocalAI gallery job (`kind` "backends" or "models"): applied, then
    polled to its end with the bytes it reports as the progress."""
    status, body = localai.request("POST", "%s/%s/apply" % (server_url, kind), request)
    if status != 200:
        raise Failed("%s: %s" % (label, localai.error_message(status, body)))
    try:
        reply = json.loads(body.decode("utf-8"))
        uuid = reply.get("uuid") or reply.get("id")
    except ValueError:
        uuid = None
    if not uuid:
        raise Failed("%s: LocalAI did not start the installation." % label)
    url = "%s/%s/jobs/%s" % (server_url, kind, uuid)
    while True:
        try:
            check_cancel()
        except Cancelled:
            localai.request("POST", url + "/cancel", {}, timeout=5)  # best effort: a server of our own stops anyway
            raise
        status, body = localai.request("GET", url)
        if status != 200:
            raise Failed("%s: %s" % (label, localai.error_message(status, body)))
        try:
            job = json.loads(body.decode("utf-8"))
        except ValueError:
            job = {}
        failure = job_failure(job)
        if failure:
            raise Failed("%s: %s" % (label, failure))
        if job.get("cancelled"):
            raise Failed("%s was cancelled." % label)
        if job.get("processed"):
            return
        total = job.get("total_bytes")
        if isinstance(total, (int, float)) and total > 0:
            progress.set("downloading", detail=label, done=int(job.get("current_bytes") or 0), total=int(total))
        else:
            progress.set("installing", detail=label)
        time.sleep(JOB_POLL)


def watch_growth(c, progress, stop):
    """A step with no byte count (a backend fetching its weights on first
    use): what the component's folders grow to against its size."""
    total = int(c.get("size") or 0)
    paths = model_paths(c)
    # What another component holds (the shared backend and cache) is not
    # downloaded now: the growth counts from here, against what is left.
    start = 0
    if any(shared_with(c, r) for r in c.get("paths", [])):
        start = sum(tree_size(p) for p in paths)
        total = max(0, total - int(c.get("shared_size") or 0))

    def loop():
        while not stop.wait(1.0):
            size = sum(tree_size(p) for p in paths) - start
            progress.set(done=min(size, int(total * 0.99)) if total else size, total=total or None)
    t = threading.Thread(target=loop, daemon=True)
    t.start()
    return t


def backend_present(c):
    d = c.get("backend_dir")
    return not d or os.path.isdir(os.path.join(localai.backends_dir(), d))


def install_model(c, progress):
    needs = find(c.get("requires") or "")
    if needs and not is_installed(needs):
        raise Failed("Install %s first (Settings > Components)." % needs["title"])
    server = localai.Server()
    CHILD["server"] = server
    try:
        progress.set("installing", detail="Starting LocalAI", force=True)
        why = server.ensure(should_stop=CANCEL.is_set)
        check_cancel()
        if why:
            raise Failed(why)
        url = server.url
        if not backend_present(c):
            gallery_job(url, "backends", {"id": c["backend"]}, progress, "Backend " + c["backend"].split("@")[-1])
            if not backend_present(c):
                raise Failed("The backend is not there after its installation.")
        request = dict(c["model"])
        request["overrides"] = c["model"].get("overrides") or {}
        request["id"] = request.pop("gallery")
        gallery_job(url, "models", request, progress, "Voice " + model_name(c))
        if not os.path.isfile(model_yaml(c)):
            raise Failed("LocalAI did not write the model's configuration.")
        warm = c.get("warmup")
        if warm:
            progress.set("installing", detail="Loading it for the first time", force=True)
            stop = threading.Event()
            watch_growth(c, progress, stop)
            answer = []
            body = {"model": model_name(c), "input": warm}
            worker = threading.Thread(
                target=lambda: answer.append(localai.request("POST", url + "/v1/audio/speech", body, timeout=1800)),
                daemon=True)
            worker.start()
            try:
                while worker.is_alive():
                    check_cancel()  # the server stopping below ends the request
                    worker.join(0.3)
            finally:
                stop.set()
            status, reply = answer[0] if answer else (0, b"")
            if status != 200:
                raise Failed("It did not load: %s" % localai.error_message(status, reply))
    finally:
        CHILD["server"] = None
        server.stop()
    os.makedirs(os.path.dirname(marker_path(c)), exist_ok=True)
    tmp = marker_path(c) + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump({"id": c["id"], "installed": time.time(), "model": model_name(c)}, f)
    os.replace(tmp, marker_path(c))


# ----------------------------------------------------------------- system

def polkit_agent():
    forced = os.environ.get("NOTCHD_POLKIT_AGENT")
    if forced is not None:
        return forced == "1"
    pattern = re.compile(r"polkit.*(agent|gnome|kde|mate)|policykit|hyprpolkitagent|lxpolkit|soteria|gnome-shell|plasmashell")
    for name in os.listdir("/proc"):
        if not name.isdigit():
            continue
        try:
            with open("/proc/%s/cmdline" % name, "rb") as f:
                cmdline = f.read().replace(b"\0", b" ").decode(errors="replace")
        except OSError:
            continue
        if pattern.search(cmdline.split(" ")[0]):
            return True
    return False


def terminal_command():
    forced = os.environ.get("NOTCHD_TERMINAL")
    if forced:
        return shlex.split(forced)
    if shutil.which("xdg-terminal-exec"):
        return ["xdg-terminal-exec"]
    for t in [os.environ.get("TERMINAL", "")] + ["ghostty", "alacritty", "kitty", "foot", "wezterm", "konsole", "xterm"]:
        if t and shutil.which(t.split()[0]):
            return shlex.split(t) + ["-e"]
    return None


def aur_helper():
    for h in ("yay", "paru"):
        if shutil.which(h):
            return h
    return None


def install_system(c, progress):
    missing = missing_packages(c)
    if not missing:
        return
    if not shutil.which("pacman"):
        raise Failed("Install %s with your package manager." % ", ".join(missing))
    aur = bool(c.get("aur"))
    helper = aur_helper() if aur else None
    if aur and not helper:
        raise Failed("%s is in the AUR: install it with an AUR helper, for example  yay -S %s" % (c["title"], " ".join(missing)))
    pacman = ["pacman", "-S", "--needed", "--noconfirm"] + missing
    if not aur and shutil.which("pkexec") and polkit_agent():
        progress.set("installing", detail="Waiting for you to authenticate", force=True)
        try:
            run(["pkexec"] + pacman, progress)
        except Failed as e:
            if re.search(r"\((126|127)\)", str(e)):
                raise Failed("Authentication was cancelled or refused.")
            raise
    else:
        term = terminal_command()
        if not term:
            raise Failed("No polkit agent or terminal to ask for your password: run  sudo pacman -S --needed %s" % " ".join(missing))
        os.makedirs(state_dir(), exist_ok=True)
        done_file = os.path.join(state_dir(), c["id"] + ".exit")
        if os.path.exists(done_file):
            os.unlink(done_file)
        command = ("%s -S --needed" % helper) if aur else "sudo pacman -S --needed"
        script = ("%s %s; echo $? > %s; echo; printf 'Press Enter to close'; read _"
                  % (command, " ".join(shlex.quote(p) for p in missing), shlex.quote(done_file)))
        progress.set("installing", detail="Waiting for the terminal that asks for your password", force=True)
        try:
            proc = subprocess.Popen(term + ["sh", "-c", script], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL, start_new_session=True)
        except OSError as e:
            raise Failed("Could not open a terminal: %s." % e)
        CHILD["proc"] = proc  # cancelling closes the terminal
        deadline = time.time() + 1800
        while not os.path.exists(done_file):
            check_cancel()
            code = proc.poll()
            if code not in (None, 0):
                raise Failed("The terminal could not open (%d)." % code)
            if time.time() > deadline:
                raise Failed("Gave up waiting for the terminal.")
            time.sleep(0.2)
        CHILD["proc"] = None
        try:
            with open(done_file) as f:
                code = int((f.read() or "1").strip())
        except (OSError, ValueError):
            code = 1
        os.unlink(done_file)
        if code != 0:
            raise Failed("%s failed (%d); see the terminal." % (helper or "pacman", code))
    progress.set(detail="Checking", force=True)
    left = missing_packages(c)
    if left:
        raise Failed("Not installed: %s." % ", ".join(left))


# ----------------------------------------------------------------- the job

def clean(c, keep_partials):
    """Cancelled: what was started goes. A failure keeps its partials."""
    if keep_partials:
        return
    if c["kind"] == "files":
        for p in file_paths(c):
            if os.path.exists(p + ".part"):
                os.unlink(p + ".part")
    elif c["kind"] == "localai" and not os.path.isfile(marker_path(c)):
        forget(c)
        delete(model_yaml(c))
        delete(gallery_note(c))


def run_job(cid):
    c = find(cid)
    if not c:
        print("no component " + cid, file=sys.stderr)
        return 2
    signal.signal(signal.SIGTERM, on_term)
    signal.signal(signal.SIGINT, on_term)
    progress = Progress(cid)
    progress.set("queued", force=True)
    try:
        if not is_installed(c):
            {"files": install_files, "localai": install_model, "system": install_system}[c["kind"]](c, progress)
        progress.finish("done")
        return 0
    except Cancelled:
        clean(c, keep_partials=False)
        progress.finish("cancelled")
        return 130
    except Failed as e:
        clean(c, keep_partials=True)
        progress.fail(str(e))
        return 1
    except Exception as e:  # nothing here may leave the state saying it still runs
        clean(c, keep_partials=True)
        progress.fail("%s: %s" % (type(e).__name__, e))
        return 1


def start(cid):
    """Starts the job detached, so closing the settings (or the shell) does
    not stop it."""
    c = find(cid)
    if not c:
        print("no component " + cid, file=sys.stderr)
        return 2
    if is_active(read_state(cid)):
        return 0
    write_state(cid, phase="queued", detail="", done=None, total=None, error="", pid=0)
    subprocess.Popen([sys.executable, os.path.abspath(__file__), "run", cid], stdin=subprocess.DEVNULL,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True, close_fds=True)
    return 0


def cancel(cid):
    st = read_state(cid)
    pid = st.get("pid")
    if is_active(st) and pid:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
        return 0
    c = find(cid)
    if c and st.get("phase") in ACTIVE:  # a worker that is gone
        clean(c, keep_partials=False)
        write_state(cid, phase="cancelled", detail="", done=None, total=None, error="", pid=0)
    return 0


def delete(p):
    for q in (p, p + ".partial", p + ".install-tmp"):
        if os.path.isdir(q):
            shutil.rmtree(q, ignore_errors=True)
        elif os.path.exists(q):
            os.unlink(q)


def forget(c):
    """Deletes what a LocalAI component's `paths` declare, except what another
    installed component holds too (its backend, the weights cache): of the
    cache, only this one's language goes, and the blobs no one uses."""
    for rel, p in zip(c.get("paths", []), model_paths(c)):
        if shared_with(c, rel):
            if rel == "hf" and c.get("hf_language"):
                hf_forget(c["hf_language"])
            continue
        delete(p)


def remove(cid):
    c = find(cid)
    if not c:
        print("no component " + cid, file=sys.stderr)
        return 2
    if c["kind"] == "system":
        print("%s is installed by the system: remove it with your package manager." % c["title"], file=sys.stderr)
        return 1
    if is_active(read_state(cid)):
        print("%s is being installed: cancel it first." % c["title"], file=sys.stderr)
        return 1
    if c["kind"] == "localai":
        forget(c)
        for p in [model_yaml(c), gallery_note(c), marker_path(c)]:
            delete(p)
    elif c.get("remove_dir"):
        d = os.path.realpath(component_dir(c))
        if os.path.dirname(d) != os.path.realpath(data_root()):  # only a folder of notchd's own
            print("refusing to remove " + d, file=sys.stderr)
            return 1
        if localai.reachable(localai.base_url()):
            print("LocalAI is running (a conversation?): stop it first.", file=sys.stderr)
            return 1
        shutil.rmtree(d, ignore_errors=True)
    else:
        for p in file_paths(c):
            for q in (p, p + ".part"):
                if os.path.exists(q):
                    os.unlink(q)
    clear_state(cid)
    return 0


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if not argv or argv[0] not in ("status", "install", "run", "cancel", "remove"):
        print(__doc__, file=sys.stderr)
        return 2
    cmd = argv[0]
    if cmd == "status":
        print(json.dumps(status()))
        return 0
    if len(argv) != 2:
        print("%s needs a component id" % cmd, file=sys.stderr)
        return 2
    return {"install": start, "run": run_job, "cancel": cancel, "remove": remove}[cmd](argv[1])


if __name__ == "__main__":
    sys.exit(main())
