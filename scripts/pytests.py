#!/usr/bin/env python3
"""Runs the Python tests (tests/*_test.py) of one layer of the test pyramid,
in parallel, and records what each test cost.

  scripts/pytests.py unit|integration [-j N] [file ...]

A TestCase is a unit test unless its class (or a base) says
`tier = "integration"`: it starts processes or servers. Unit tests run in
one worker. Integration tests are spread over N workers (default: the CPUs,
at most 8), longest first by the last run's times, each worker in its own
process group, killed with whatever it left behind when it ends.

Each test's seconds go to build/times/py-<tier>.tsv (seconds, test), read by
scripts/test-times.py. Exits 1 on a failure, an error, or a layer with no
tests (a class put in the wrong layer by mistake).
"""
import importlib.util
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import traceback
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TIMES = os.path.join(ROOT, "build", "times")
WORKER_TIMEOUT = 300


def load(path):
    name = os.path.splitext(os.path.basename(path))[0]
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def tests_of(path, tier):
    """The test names ("Class.method") of `path` in `tier`."""
    module = load(path)
    loader = unittest.TestLoader()
    names = []
    for attr in sorted(dir(module)):
        cls = getattr(module, attr)
        if not (isinstance(cls, type) and issubclass(cls, unittest.TestCase)):
            continue
        if getattr(cls, "tier", "unit") != tier:
            continue
        names += ["%s.%s" % (attr, m) for m in loader.getTestCaseNames(cls)]
    return names


# ------------------------------------------------------------------ worker

class Timed(unittest.TestResult):
    def __init__(self):
        super().__init__()
        self.rows = {}

    def startTest(self, test):
        self.started = time.monotonic()
        super().startTest(test)

    def stopTest(self, test):
        super().stopTest(test)
        row = self.rows.setdefault(test.id(), {"status": "ok", "detail": ""})
        row["seconds"] = time.monotonic() - self.started

    def note(self, test, status, err):
        self.rows[test.id()] = {"status": status, "detail": self._exc_info_to_string(err, test)}

    def addError(self, test, err):
        if isinstance(test, unittest.TestCase):
            self.note(test, "error", err)
        else:  # a setUpClass or module error: no test to pin it on
            self.rows[str(test)] = {"status": "error", "seconds": 0, "detail": self._exc_info_to_string(err, test)}

    def addFailure(self, test, err):
        self.note(test, "fail", err)

    def addSubTest(self, test, subtest, err):
        if err is not None:
            status = "fail" if issubclass(err[0], test.failureException) else "error"
            self.note(test, status, err)


def worker(out, items):
    """Runs `items` ("path::Class.method") and writes their rows to `out`."""
    rows = []
    by_file = {}
    for item in items:
        path, name = item.split("::", 1)
        by_file.setdefault(path, []).append(name)
    for path, names in by_file.items():
        short = os.path.relpath(path, ROOT)
        try:
            module = load(path)
            suite = unittest.TestLoader().loadTestsFromNames(names, module)
        except Exception:
            rows.append({"id": short, "status": "error", "seconds": 0, "detail": traceback.format_exc()})
            continue
        result = Timed()
        suite.run(result)
        for tid, row in result.rows.items():
            row["id"] = "%s::%s" % (short, tid.split(".", 1)[-1])
            rows.append(row)
    with open(out, "w") as f:
        json.dump(rows, f)


# -------------------------------------------------------------------- main

def last_times(tier):
    times = {}
    try:
        with open(os.path.join(TIMES, "py-%s.tsv" % tier)) as f:
            for line in f:
                secs, tid = line.rstrip("\n").split("\t", 1)
                times[tid] = float(secs)
    except (OSError, ValueError):
        pass
    return times


def schedule(items, jobs, times):
    """Longest first, each to the least loaded worker."""
    def cost(item):
        path, name = item.split("::", 1)
        return times.get("%s::%s" % (os.path.relpath(path, ROOT), name), 1.0)
    bins = [[0.0, []] for _ in range(jobs)]
    for item in sorted(items, key=cost, reverse=True):
        b = min(bins, key=lambda b: b[0])
        b[0] += cost(item)
        b[1].append(item)
    return [b[1] for b in bins if b[1]]


def main(argv):
    if argv[:1] == ["--worker"]:
        worker(argv[1], argv[2:])
        return 0
    tier = argv[0] if argv else ""
    if tier not in ("unit", "integration"):
        print(__doc__.strip(), file=sys.stderr)
        return 2
    args = argv[1:]
    jobs = min(os.cpu_count() or 1, 8)
    if args[:1] == ["-j"]:
        jobs, args = max(1, int(args[1])), args[2:]
    files = [os.path.abspath(f) for f in args] or sorted(
        os.path.join(ROOT, "tests", n) for n in os.listdir(os.path.join(ROOT, "tests")) if n.endswith("_test.py"))

    items = ["%s::%s" % (f, n) for f in files for n in tests_of(f, tier)]
    if not items:
        print("py %s: no tests" % tier, file=sys.stderr)
        return 1
    groups = schedule(items, 1 if tier == "unit" else jobs, last_times(tier))

    start = time.monotonic()
    scratch = tempfile.mkdtemp(prefix="notchd-pytests.")
    procs = []
    try:
        for i, group in enumerate(groups):
            out = os.path.join(scratch, "%d.json" % i)
            p = subprocess.Popen([sys.executable, os.path.abspath(__file__), "--worker", out] + group,
                                 start_new_session=True, stdout=subprocess.DEVNULL)
            procs.append((p, out))
        rows = []
        deadline = time.monotonic() + WORKER_TIMEOUT
        for p, out in procs:
            try:
                p.wait(timeout=max(1, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                rows.append({"id": "worker %d" % p.pid, "status": "error", "seconds": 0,
                             "detail": "still running after %ds; killed" % WORKER_TIMEOUT})
            try:
                with open(out) as f:
                    rows += json.load(f)
            except (OSError, ValueError):
                if p.returncode is not None:
                    rows.append({"id": "worker %d" % p.pid, "status": "error", "seconds": 0,
                                 "detail": "exited %d without results" % p.returncode})
    finally:
        for p, _ in procs:  # the worker and anything a test left running in its group
            try:
                os.killpg(p.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            p.wait()
        for n in os.listdir(scratch):
            os.unlink(os.path.join(scratch, n))
        os.rmdir(scratch)
    wall = time.monotonic() - start

    bad = [r for r in rows if r["status"] != "ok"]
    for r in bad:
        print("%s %s\n%s" % (r["status"].upper(), r["id"], r["detail"]), file=sys.stderr)
    os.makedirs(TIMES, exist_ok=True)
    with open(os.path.join(TIMES, "py-%s.tsv" % tier), "w") as f:
        for r in sorted(rows, key=lambda r: -r["seconds"]):
            f.write("%.3f\t%s\n" % (r["seconds"], r["id"]))
    ran = len([r for r in rows if r["id"].count("::")])
    print("py %s: %d tests, %d failed, %.1fs on %d worker%s" % (
        tier, ran, len(bad), wall, len(groups), "" if len(groups) == 1 else "s"))
    return 1 if bad or ran < len(items) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
