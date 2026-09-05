#!/usr/bin/env python3
"""Rank the `src/**` files below the floor by how many lines each one is short.

`scripts/coverage-check.sh` sorts by percentage, which puts a 60-line file at
40% ahead of a 4,000-line file at 96% — the opposite of the order the work
should be done in. This ranks by uncovered lines and prints the size
distribution, so "how much is left" is a number rather than a feeling.

  usage: coverage-src-gaps.py [report.json] [--top N]

Applies scripts/coverage-exceptions.txt exactly as coverage-check.sh does.
`src/**` only, for the reason given in scripts/coverage-src-delta.py.
"""
import json
import os
import sys

FLOOR = float(os.environ.get("FLOOR", "98"))


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    report = args[0] if args else "target/coverage/coverage.json"
    top = 25
    if "--top" in sys.argv:
        top = int(sys.argv[sys.argv.index("--top") + 1])

    skip = set()
    path = "scripts/coverage-exceptions.txt"
    if os.path.exists(path):
        for line in open(path):
            line = line.split("#", 1)[0].strip()
            if line:
                skip.add(line)

    with open(report) as handle:
        data = json.load(handle)

    rows = []
    for entry in data["data"][0]["files"]:
        name = entry["filename"]
        at = name.rfind("/src/")
        if at < 0:
            continue
        name = name[at + 1 :]
        if not name.startswith("src/") or name in skip:
            continue
        lines = entry["summary"]["lines"]
        if lines["count"] == 0 or lines["percent"] >= FLOOR:
            continue
        rows.append(
            (lines["count"] - lines["covered"], lines["percent"], lines["covered"], lines["count"], name)
        )

    rows.sort(reverse=True)
    print(f"{len(rows)} src/** files below {FLOOR:.0f}%, {sum(r[0] for r in rows)} uncovered lines")

    buckets = {"1-3": 0, "4-10": 0, "11-30": 0, "31-100": 0, "101+": 0}
    for short, *_ in rows:
        key = (
            "1-3" if short <= 3 else
            "4-10" if short <= 10 else
            "11-30" if short <= 30 else
            "31-100" if short <= 100 else
            "101+"
        )
        buckets[key] += 1
    print("  by lines short: " + "  ".join(f"{k}: {v}" for k, v in buckets.items()))

    print(f"\n--- {top} largest gaps")
    for short, pct, covered, total, name in rows[:top]:
        print(f"{short:5d} short  {pct:6.2f}%  ({covered}/{total})  {name}")


if __name__ == "__main__":
    main()
