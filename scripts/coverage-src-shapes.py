#!/usr/bin/env python3
"""Classify every uncovered `src/**` line by what its source text looks like.

    python3 scripts/coverage-src-shapes.py <report.json>

Answers "what IS the remaining gap" with a count per shape rather than an
impression, and prints three examples of each so a shape can be checked rather
than taken on trust. `coverage-src-gaps.py` says which FILE to work on and
`coverage-src-lines.py` says which LINES; this says what KIND of thing they are,
which is what decides whether the next move is a program, a sweep, or an
exception.

A line is attributed to the first pattern that matches, so the order of
PATTERNS is the priority order: `unreachable!` before `panic!`, and both before
the `?` that may sit on the same line.

Counts region-ENTRY lines only, so the total is smaller than the `N short` in
`coverage-src-gaps.py` (one region can span several lines). Applies
`scripts/coverage-exceptions.txt` exactly as `coverage-check.sh` does, and skips
`repository/src/**` for the reason in `coverage-src-delta.py`.
"""
import json
import re
import sys

report = sys.argv[1]

skip = set()
for line in open("scripts/coverage-exceptions.txt"):
    line = line.split("#", 1)[0].strip()
    if line:
        skip.add(line)

PATTERNS = [
    # No trailing `\b`: the boundary after `!` never matches, because `!` and the
    # `(` that follows it are both non-word characters. With one there, every
    # `unreachable!(...)` fell through to "other" -- 51 lines misfiled, and the
    # bucket that decides "exception or test" read as the bucket that means
    # "write a program".
    ("unreachable!/todo!", re.compile(r"\b(unreachable!|todo!|unimplemented!)")),
    ("panic!/expect", re.compile(r"(\bpanic!|\.expect\()")),
    ("`?` on a call", re.compile(r"\)\?;?\s*$|\?;\s*$")),
    ("return Err(...)", re.compile(r"\breturn Err\(")),
    ("Err(...) tail", re.compile(r"^\s*Err\(")),
    ("None / Ok(None) tail", re.compile(r"^\s*(None|Ok\(None\)),?\s*$")),
    ("else / closing brace", re.compile(r"^\s*(\}|\} else \{|\})\s*$")),
    ("match arm", re.compile(r"=>")),
    ("if / let-else guard", re.compile(r"^\s*(if |let .*else|else if )")),
    ("continue / break", re.compile(r"^\s*(continue|break)\b")),
]

counts = {name: 0 for name, _ in PATTERNS}
counts["other"] = 0
examples = {name: [] for name in counts}

data = json.load(open(report))
total = 0
for entry in data["data"][0]["files"]:
    name = entry["filename"]
    if "/repository/src/" in name:
        continue
    at = name.rfind("/src/")
    if at < 0:
        continue
    key = name[at + 1 :]
    if not key.startswith("src/") or key in skip:
        continue
    lines = entry["summary"]["lines"]
    if lines["count"] == 0 or lines["percent"] >= 98:
        continue

    seen, covered = set(), set()
    for line, _c, count, has, is_entry, _g in entry["segments"]:
        if has and is_entry:
            seen.add(line)
            if count > 0:
                covered.add(line)
    try:
        src = open(name).read().splitlines()
    except OSError:
        continue
    for n in sorted(seen - covered):
        if n > len(src):
            continue
        text = src[n - 1]
        total += 1
        for label, pattern in PATTERNS:
            if pattern.search(text):
                counts[label] += 1
                if len(examples[label]) < 3:
                    examples[label].append(f"{key}:{n}  {text.strip()[:80]}")
                break
        else:
            counts["other"] += 1
            if len(examples["other"]) < 6:
                examples["other"].append(f"{key}:{n}  {text.strip()[:80]}")

print(f"{total} uncovered region-entry lines across the files below the floor\n")
for label, count in sorted(counts.items(), key=lambda kv: -kv[1]):
    if not count:
        continue
    print(f"  {count:5}  {100 * count / total:5.1f}%  {label}")
    for example in examples[label]:
        print(f"           {example}")
