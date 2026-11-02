#!/usr/bin/env python3
"""Runs a command and records what it cost: wall-clock seconds and the CPU
seconds of everything it ran (its processes, once reaped).

  scripts/timed.py <name> <command> [args...]

Writes build/times/<name>.cost ("wall cpu", seconds) for
scripts/test-times.py, and exits with the command's code.
"""
import os
import resource
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def main(argv):
    if len(argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    name, cmd = argv[0], argv[1:]
    start = time.monotonic()
    try:
        code = subprocess.call(cmd)
    except KeyboardInterrupt:
        return 130
    wall = time.monotonic() - start
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    times = os.path.join(ROOT, "build", "times")
    os.makedirs(times, exist_ok=True)
    with open(os.path.join(times, name + ".cost"), "w") as f:
        f.write("%.3f %.3f\n" % (wall, usage.ru_utime + usage.ru_stime))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
