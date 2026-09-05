#!/usr/bin/env python3
"""Compare two `cargo llvm-cov ... --json` reports over `src/**` only.

Coverage of `src/**` is a pure function of the `--bins` unit tests (`mfb` is a
binary-only package, so nothing under `tests/` links it), which is what makes a
`scripts/coverage-bins.sh` report comparable with a full `scripts/coverage.sh`
one for these files -- and only for these files. `repository/src/**` is
deliberately excluded: its crate IS a library, so its integration tests do
contribute and a bins-only run understates it.

  usage: coverage-src-delta.py <baseline.json> [current.json]

With one argument it just reports; with two it prints the per-file delta of
every file whose percentage moved, plus the below-floor counts on each side.
Files listed in scripts/coverage-exceptions.txt are excluded from the counts,
exactly as scripts/coverage-check.sh does.
"""
import json
import os
import sys

FLOOR = float(os.environ.get("FLOOR", "98"))


def exceptions():
    out = set()
    path = "scripts/coverage-exceptions.txt"
    if not os.path.exists(path):
        return out
    with open(path) as handle:
        for line in handle:
            line = line.split("#", 1)[0].strip()
            if line:
                out.add(line)
    return out


def load(path):
    with open(path) as handle:
        data = json.load(handle)
    rows = {}
    for entry in data["data"][0]["files"]:
        name = entry["filename"]
        # The report carries absolute paths and the two runs may come from
        # different worktrees, so key on the repo-relative tail.
        at = name.rfind("/src/")
        if at < 0:
            continue
        name = name[at + 1 :]
        if not name.startswith("src/"):
            continue
        lines = entry["summary"]["lines"]
        if lines["count"] == 0:
            continue
        rows[name] = (lines["percent"], lines["covered"], lines["count"])
    return rows


def below(rows, skip):
    return {n: v for n, v in rows.items() if v[0] < FLOOR and n not in skip}


def main():
    skip = exceptions()
    base = load(sys.argv[1])
    if len(sys.argv) < 3:
        under = below(base, skip)
        for name, (pct, cov, total) in sorted(under.items(), key=lambda kv: kv[1]):
            print(f"{pct:6.2f}%  ({cov}/{total})  {name}")
        print(f"\n{len(under)} of {len(base)} src/** files below {FLOOR:.0f}%")
        return

    cur = load(sys.argv[2])
    moved = [
        (cur[n][0] - base[n][0], n, base[n], cur[n])
        for n in sorted(set(base) & set(cur))
        if abs(cur[n][0] - base[n][0]) > 0.005
    ]
    for delta, name, b, c in sorted(moved):
        print(f"{delta:+7.2f}  {b[0]:6.2f}% -> {c[0]:6.2f}%  ({c[1]}/{c[2]})  {name}")
    added = sorted(set(cur) - set(base))
    for name in added:
        print(f"    new  {cur[name][0]:6.2f}%  ({cur[name][1]}/{cur[name][2]})  {name}")
    print(
        f"\nbelow {FLOOR:.0f}%: {len(below(base, skip))} -> {len(below(cur, skip))}"
        f"   ({len(moved)} files moved, {len(added)} new)"
    )


if __name__ == "__main__":
    main()
