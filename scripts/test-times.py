#!/usr/bin/env python3
"""What the tests cost, layer by layer of the pyramid, from the last run of
each layer (build/times/, written by the make targets).

  scripts/test-times.py        (make times)

Per layer: how many tests, the wall-clock time and CPU time of the run, and
the mean and slowest test. Then the slowest tests overall, and those over
their layer's budget: a unit test that takes long is not a unit test (make it
cheaper or move it up); a case over the top layers' budget is worth a look.
Writes the same report to build/test-times.md.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TIMES = os.path.join(ROOT, "build", "times")

# (layer, make target, per-test files, cost files, budget per test in s)
LAYERS = [
    ("unit", "make test", ["node.tsv", "py-unit.tsv"], ["node", "py-unit"], 0.1),
    ("integration", "make integration", ["py-integration.tsv"], ["py-integration"], 5.0),
    ("qml", "make qmltest", ["qml.tsv"], ["qml"], 5.0),
    ("visual", "make visual", [], ["visual"], None),
]
STATIC = [("lint", "make lint"), ("validate", "make validate")]


def rows(name):
    out = []
    try:
        with open(os.path.join(TIMES, name)) as f:
            for line in f:
                secs, test = line.rstrip("\n").split("\t", 1)
                out.append((float(secs), test))
    except (OSError, ValueError):
        return None
    return out


def cost(name):
    try:
        with open(os.path.join(TIMES, name + ".cost")) as f:
            wall, cpu = f.read().split()
            return float(wall), float(cpu)
    except (OSError, ValueError):
        return None


def visual_scenarios():
    try:
        with open(os.path.join(ROOT, "tests", "visual", "shell.qml")) as f:
            return len(re.findall(r'name: "[a-z-]+"', f.read()))
    except OSError:
        return 0


def fmt(secs):
    return "%.2fs" % secs if secs < 10 else "%.0fs" % secs


def main():
    lines = ["# Test pyramid: what each layer costs", ""]
    lines += ["| layer | run with | tests | wall | CPU | mean | slowest |", "|---|---|---:|---:|---:|---:|---:|"]
    everything, over, missing = [], [], []
    total_wall = total_cpu = 0.0
    for layer, target, tsvs, costs, budget in LAYERS:
        tests = []
        for t in tsvs:
            got = rows(t)
            if got is None:
                missing.append(target)
            else:
                tests += got
        walls = [c for c in map(cost, costs) if c]
        if len(walls) < len(costs) and target not in missing:
            missing.append(target)
        wall = sum(w for w, _ in walls)
        cpu = sum(c for _, c in walls)
        total_wall += wall
        total_cpu += cpu
        n = len(tests) if tsvs else visual_scenarios()
        mean = sum(s for s, _ in tests) / len(tests) if tests else (wall / n if n else 0)
        slowest = max((s for s, _ in tests), default=0)
        lines.append("| %s | `%s` | %d | %s | %s | %s | %s |" % (
            layer, target, n, fmt(wall), fmt(cpu), fmt(mean), fmt(slowest) if tests else "-"))
        everything += [(s, layer, t) for s, t in tests]
        if budget is not None:
            over += [(s, layer, t, budget) for s, t in tests if s > budget]
    for name, target in STATIC:
        c = cost(name)
        if c is None:
            missing.append(target)
            continue
        total_wall += c[0]
        total_cpu += c[1]
        lines.append("| static: %s | `%s` | - | %s | %s | - | - |" % (name, target, fmt(c[0]), fmt(c[1])))
    lines += ["", "Sum of the layers: %s wall, %s CPU (`make check` runs the top three at once)." % (
        fmt(total_wall), fmt(total_cpu)), ""]

    lines += ["## Slowest tests", "", "| seconds | layer | test |", "|---:|---|---|"]
    for s, layer, t in sorted(everything, reverse=True)[:15]:
        lines.append("| %.2f | %s | %s |" % (s, layer, t))
    lines.append("")
    if over:
        lines += ["## Over budget", ""]
        for s, layer, t, budget in sorted(over, reverse=True):
            lines.append("- %.2fs > %gs (%s): %s" % (s, budget, layer, t))
        lines.append("")
    if missing:
        lines += ["Not measured yet: run %s." % ", ".join("`%s`" % m for m in dict.fromkeys(missing)), ""]

    report = "\n".join(lines)
    os.makedirs(TIMES, exist_ok=True)
    with open(os.path.join(ROOT, "build", "test-times.md"), "w") as f:
        f.write(report)
    print(report)
    return 0


if __name__ == "__main__":
    sys.exit(main())
