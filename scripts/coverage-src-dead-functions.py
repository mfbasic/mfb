#!/usr/bin/env python3
"""Rank the `src/**` functions that never ran, by how many lines they span.

    python3 scripts/coverage-src-dead-functions.py <report.json> [--top N]

The other three instruments work in lines: `coverage-src-gaps.py` ranks files,
`coverage-src-lines.py` ranks the lines inside one, `coverage-src-shapes.py`
says what kind of thing they are. None of them says the most actionable thing,
which is **which whole functions never ran at all** — and that is a different
question with a much better answer, because one program usually reaches a whole
function where nothing reaches a scattered guard.

Ranked by span rather than by count, because a 90-line function nothing calls is
one program away from being covered and a 3-line one is rarely worth a fixture.

Only counts a function whose execution_count is 0 across EVERY instantiation:
a generic that ran for one type argument and not another is not dead, it is
partly covered, and its uncovered lines belong to `coverage-src-lines.py`.
"""
import json
import os
import sys
from collections import defaultdict


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    report = args[0] if args else "target/coverage/coverage.json"
    top = 30
    if "--top" in sys.argv:
        top = int(sys.argv[sys.argv.index("--top") + 1])

    skip = set()
    path = "scripts/coverage-exceptions.txt"
    if os.path.exists(path):
        for line in open(path):
            line = line.split("#", 1)[0].strip()
            if line:
                skip.add(line)

    data = json.load(open(report))

    # A function can appear once per instantiation; sum the counts before
    # judging it dead.
    total_count: dict[tuple[str, str], int] = defaultdict(int)
    span: dict[tuple[str, str], int] = {}
    for record in data["data"][0]["functions"]:
        filenames = record.get("filenames") or []
        if not filenames:
            continue
        name = record["name"]
        for filename in filenames:
            key = (filename, name)
            total_count[key] += record.get("count", 0)
            # regions are [start_line, start_col, end_line, end_col, count, ...]
            lines = {r[0] for r in record.get("regions", [])}
            lines |= {r[2] for r in record.get("regions", [])}
            if lines:
                span[key] = max(span.get(key, 0), max(lines) - min(lines) + 1)

    rows = []
    for (filename, name), count in total_count.items():
        if count:
            continue
        if "/repository/src/" in filename:
            continue
        at = filename.rfind("/src/")
        if at < 0:
            continue
        short = filename[at + 1 :]
        if not short.startswith("src/") or short in skip:
            continue
        rows.append((span.get((filename, name), 0), short, name))

    rows.sort(reverse=True)
    print(f"{len(rows)} src/** functions never executed, spanning {sum(r[0] for r in rows)} lines")
    print(f"\n--- {top} widest")
    for width, short, name in rows[:top]:
        print(f"  {width:5} lines  {short}\n                {demangle(name)}")
    return 0


def demangle(name: str) -> str:
    """Strip the crate-hash prefix a Rust symbol carries, keeping the path."""
    if "17h" in name:
        name = name.rsplit("17h", 1)[0]
    return name.replace("..", "::")


if __name__ == "__main__":
    sys.exit(main())
