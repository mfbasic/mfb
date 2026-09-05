#!/usr/bin/env python3
"""Print the uncovered line ranges of one `src/**` file from a llvm-cov JSON export.

    python3 scripts/coverage-src-lines.py <report.json> <path-suffix> [--source]

`<path-suffix>` is matched against the end of each `filename` in the report, so
`ir/shape.rs` and the full absolute path both work. `--source` interleaves the
source text, which is how you tell a genuinely-untested branch from a `?` arm
that cannot fail.

Coverage is reported per *region*, and several regions can share a line. A line
counts as uncovered here only when EVERY region touching it has a zero count —
the same rule the per-file gate applies, so the ranges printed add up to the
`N short` figure `coverage-src-gaps.py` ranks by.
"""
import json
import sys


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    report, suffix = sys.argv[1], sys.argv[2]
    want_source = "--source" in sys.argv[3:]

    data = json.load(open(report))
    matches = [
        f for f in data["data"][0]["files"] if f["filename"].endswith(suffix)
    ]
    if len(matches) != 1:
        for f in matches:
            print(f["filename"], file=sys.stderr)
        print(f"{len(matches)} files match {suffix!r}; be more specific", file=sys.stderr)
        return 2
    entry = matches[0]

    # A segment is (line, col, count, has_count, is_region_entry, is_gap).
    covered: set[int] = set()
    seen: set[int] = set()
    for line, _col, count, has_count, is_entry, _gap in entry["segments"]:
        if not (has_count and is_entry):
            continue
        seen.add(line)
        if count > 0:
            covered.add(line)
    missing = sorted(seen - covered)

    summary = entry["summary"]["lines"]
    print(
        f"{entry['filename']}\n"
        f"  {summary['percent']:.2f}%  ({summary['covered']}/{summary['count']})"
        f"  {summary['count'] - summary['covered']} short\n"
        f"  {len(missing)} region-entry lines with no execution"
    )

    src = None
    if want_source:
        path = entry["filename"]
        try:
            src = open(path).read().splitlines()
        except OSError as exc:
            print(f"  (source unreadable: {exc})", file=sys.stderr)

    for start, end in ranges(missing):
        label = f"{start}" if start == end else f"{start}-{end}"
        print(f"  {label}")
        if src:
            for n in range(start, min(end, len(src)) + 1):
                print(f"    {n:5}  {src[n - 1]}")
    return 0


def ranges(lines: list[int]):
    """Collapse a sorted line list into inclusive (start, end) runs."""
    run: list[int] = []
    for line in lines:
        if run and line == run[-1] + 1:
            run.append(line)
            continue
        if run:
            yield run[0], run[-1]
        run = [line]
    if run:
        yield run[0], run[-1]


if __name__ == "__main__":
    sys.exit(main())
