#!/usr/bin/env python3
"""The program scripts/bench-terminal.sh runs in the terminal it measures:
phases the bench starts by creating files in <dir>, each writing when it is
done. `ready`, then on `go-flood` a flood of colored lines (FLOOD_MB, as fast
as the terminal takes them; `flood-done` holds the seconds it took), then on
`go-anim` the whole screen redrawn 30 times a second for ANIM_SECONDS
(`anim-done` holds the frames written), then it waits to be stopped."""
import os
import sys
import time

d = sys.argv[1]
flood_mb = int(os.environ.get("FLOOD_MB", "50"))
anim_seconds = float(os.environ.get("ANIM_SECONDS", "10"))
out = sys.stdout.buffer


def wait(name):
    path = os.path.join(d, name)
    while not os.path.exists(path):
        time.sleep(0.02)


def note(name, text):
    with open(os.path.join(d, name), "w") as f:
        f.write(text)


note("ready", "")
wait("go-flood")
line = b"".join(b"\x1b[3%dm%s" % (i % 8, bytes([97 + i % 26]) * 3) for i in range(30)) + b"\x1b[0m\r\n"
block = line * 256
total, goal = 0, flood_mb << 20
t0 = time.monotonic()
while total < goal:
    out.write(block)
    total += len(block)
out.flush()
note("flood-done", f"{time.monotonic() - t0:.3f}")

wait("go-anim")
cols, rows = os.get_terminal_size()
frames = 0
t0 = time.monotonic()
while time.monotonic() - t0 < anim_seconds:
    frame = [b"\x1b[H"]
    for r in range(rows - 1):
        k = (frames + r) % 8
        frame.append(b"\x1b[3%dm" % k + (b"%06d " % (frames * rows + r) * (cols // 7)) + b"\x1b[0m\r\n")
    out.write(b"".join(frame))
    out.flush()
    frames += 1
    time.sleep(max(0.0, t0 + frames / 30 - time.monotonic()))
note("anim-done", str(frames))
while True:
    time.sleep(60)
