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

Only counts a function whose execution_count is 0 across EVERY instantiation.
A generic that ran for one type argument and not another is not dead, it is
partly covered, and its uncovered lines belong to `coverage-src-lines.py`.

That distinction is the whole difficulty here, because llvm-cov keys its records
by MANGLED name and a v0 mangled name encodes the type arguments. Summing by
mangled name reports one never-called monomorphization of a hot function as a
dead function -- which is how `copy_resource_to_current_arena` sat at the top of
this list at 197 lines while a probe showed it running ninety times per suite,
for the `&Operand` instantiation and not for another.

Records are folded by SOURCE POSITION (file plus the first line the function's
regions cover) rather than by name. Two instantiations of one function occupy
the same source lines, so the fold is exact; demangling v0 to strip the type
arguments is not, because the identifier run is interleaved with tag letters and
a regex over it silently matches nothing.
"""
import json
import os
import re
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

    # A function appears once per instantiation, under a mangled name that
    # encodes its type arguments. Fold by SOURCE POSITION so "dead" means every
    # instantiation is dead rather than some one of them.
    total_count: dict[tuple[str, int], int] = defaultdict(int)
    span: dict[tuple[str, int], int] = {}
    label: dict[tuple[str, int], str] = {}
    for record in data["data"][0]["functions"]:
        filenames = record.get("filenames") or []
        if not filenames:
            continue
        # regions are [start_line, start_col, end_line, end_col, count, ...]
        lines = {r[0] for r in record.get("regions", [])}
        lines |= {r[2] for r in record.get("regions", [])}
        if not lines:
            continue
        first, last = min(lines), max(lines)
        for filename in filenames:
            key = (filename, first)
            total_count[key] += record.get("count", 0)
            span[key] = max(span.get(key, 0), last - first + 1)
            label.setdefault(key, record["name"])

    rows = []
    for (filename, first), count in total_count.items():
        if count:
            continue
        name = f"line {first}: {label[(filename, first)]}"
        if "/repository/src/" in filename:
            continue
        at = filename.rfind("/src/")
        if at < 0:
            continue
        short = filename[at + 1 :]
        if not short.startswith("src/") or short in skip:
            continue
        rows.append((span.get((filename, first), 0), short, name))

    rows.sort(reverse=True)
    print(f"{len(rows)} src/** functions never executed, spanning {sum(r[0] for r in rows)} lines")
    print(f"\n--- {top} widest")
    for width, short, name in rows[:top]:
        print(f"  {width:5} lines  {short}\n                {demangle(name)}")
    return 0


def demangle(name: str) -> str:
    """The record's own name, as llvm-cov spells it.

    Left mangled on purpose: the fold above is by source position, so the name
    is only a pointer into the file, and a half-working demangler that dropped
    the distinguishing part would read as though it had folded when it had not.
    """
    return name


if __name__ == "__main__":
    sys.exit(main())
