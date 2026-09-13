#!/usr/bin/env python3
"""Coverage instruments over a `cargo llvm-cov ... --json` report, `src/**` only.

    coverage-report.py gaps   [report.json] [--top N]
    coverage-report.py lines  <report.json> <path-suffix> [--source]
    coverage-report.py shapes <report.json>
    coverage-report.py dead   [report.json] [--top N]
    coverage-report.py delta  <baseline.json> [current.json]

The default report is `target/coverage/coverage.json` (what `scripts/coverage.sh`
and `scripts/coverage.sh --bins` leave behind).

- `gaps` ranks the files below the floor by how many lines each one is short.
  `scripts/coverage-check.sh` sorts by percentage, which puts a 60-line file at
  40% ahead of a 4,000-line file at 96%, the opposite of the order the work should
  be done in. Also prints the size distribution.
- `lines` prints the uncovered line ranges of one file. `<path-suffix>` is matched
  against the end of each `filename`, so `ir/shape.rs` and the full absolute path
  both work. `--source` interleaves the source text, which is how you tell a
  genuinely-untested branch from a `?` arm that cannot fail. A line counts as
  uncovered only when EVERY region touching it has a zero count, the same rule
  the per-file gate applies.
- `shapes` classifies every uncovered line by what its source text looks like, with
  a count per shape and three examples of each. `gaps` says which FILE, `lines`
  says which LINES, and this says what KIND of thing they are. It counts
  region-ENTRY lines only, so its total is smaller than `gaps`' `N short`.
- `dead` ranks the functions that never ran, by how many lines they span. A
  function counts only when its execution count is 0 across EVERY instantiation:
  records are folded by SOURCE POSITION (file plus first line), not by mangled
  name, because a v0 mangled name encodes the type arguments and one never-called
  monomorphization of a hot function would otherwise read as dead.
- `delta` compares two reports: with one argument it lists the files below the
  floor; with two it prints the per-file delta of every file whose percentage
  moved, plus the below-floor counts on each side.

Why `src/**` only: `mfb` is a binary-only package, so nothing under `tests/` links
it and its coverage is a pure function of the `--bins` unit tests. That is what
makes a `coverage.sh --bins` report comparable with a full `coverage.sh` one for
these files. `repository/src/**` also ends in `/src/...` and is NOT comparable:
`mfb_repository` IS a library, so its integration tests count and a bins-only run
understates it. Its prefix is matched first and skipped everywhere.

Every subcommand applies `scripts/coverage-exceptions.txt` exactly as
`coverage-check.sh` does (`delta` applies it to the below-floor counts only). The
file is found next to this script, so the report is the same from any working
directory. `FLOOR` (default 98) comes from the environment for `gaps` and `delta`;
`shapes` fixes it at 98, as its original did.
"""
import json
import os
import re
import sys
from collections import defaultdict

SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))
EXCEPTIONS = os.path.join(SCRIPTS_DIR, "coverage-exceptions.txt")
DEFAULT_REPORT = "target/coverage/coverage.json"


def floor():
    return float(os.environ.get("FLOOR", "98"))


def exceptions():
    """The repo-relative `src/**` paths coverage-check.sh leaves out."""
    out = set()
    if not os.path.exists(EXCEPTIONS):
        return out
    with open(EXCEPTIONS) as handle:
        for line in handle:
            line = line.split("#", 1)[0].strip()
            if line:
                out.add(line)
    return out


def load(path):
    with open(path) as handle:
        return json.load(handle)["data"][0]


def src_key(filename):
    """The repo-relative `src/...` tail of a report filename, or None.

    `repository/src/**` is matched FIRST: it also ends in `/src/...`, and without
    this four of its files are silently counted as `src/**`. The report carries
    absolute paths and two runs may come from different worktrees, so callers key
    on this tail.
    """
    if "/repository/src/" in filename:
        return None
    at = filename.rfind("/src/")
    if at < 0:
        return None
    key = filename[at + 1 :]
    return key if key.startswith("src/") else None


def top_arg(default):
    if "--top" in sys.argv:
        return int(sys.argv[sys.argv.index("--top") + 1])
    return default


def positional():
    """Positional arguments after the subcommand, dropping `--flags` and `--top`'s value."""
    out, skip_next = [], False
    for arg in sys.argv[2:]:
        if skip_next:
            skip_next = False
            continue
        if arg == "--top":
            skip_next = True
            continue
        if arg.startswith("--"):
            continue
        out.append(arg)
    return out


def region_entry_lines(entry):
    """(seen, covered) sets of region-entry lines.

    A segment is (line, col, count, has_count, is_region_entry, is_gap).
    """
    seen, covered = set(), set()
    for line, _col, count, has_count, is_entry, _gap in entry["segments"]:
        if not (has_count and is_entry):
            continue
        seen.add(line)
        if count > 0:
            covered.add(line)
    return seen, covered


def ranges(lines):
    """Collapse a sorted line list into inclusive (start, end) runs."""
    run = []
    for line in lines:
        if run and line == run[-1] + 1:
            run.append(line)
            continue
        if run:
            yield run[0], run[-1]
        run = [line]
    if run:
        yield run[0], run[-1]


# --- gaps -------------------------------------------------------------------


def cmd_gaps():
    args = positional()
    report = args[0] if args else DEFAULT_REPORT
    top = top_arg(25)
    limit = floor()
    skip = exceptions()

    rows = []
    for entry in load(report)["files"]:
        name = src_key(entry["filename"])
        if name is None or name in skip:
            continue
        lines = entry["summary"]["lines"]
        if lines["count"] == 0 or lines["percent"] >= limit:
            continue
        rows.append(
            (lines["count"] - lines["covered"], lines["percent"], lines["covered"], lines["count"], name)
        )

    rows.sort(reverse=True)
    print(f"{len(rows)} src/** files below {limit:.0f}%, {sum(r[0] for r in rows)} uncovered lines")

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
    return 0


# --- lines ------------------------------------------------------------------


def cmd_lines():
    args = positional()
    if len(args) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    report, suffix = args[0], args[1]
    want_source = "--source" in sys.argv[2:]

    matches = [f for f in load(report)["files"] if f["filename"].endswith(suffix)]
    if len(matches) != 1:
        for f in matches:
            print(f["filename"], file=sys.stderr)
        print(f"{len(matches)} files match {suffix!r}; be more specific", file=sys.stderr)
        return 2
    entry = matches[0]

    seen, covered = region_entry_lines(entry)
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
        try:
            src = open(entry["filename"]).read().splitlines()
        except OSError as exc:
            print(f"  (source unreadable: {exc})", file=sys.stderr)

    for start, end in ranges(missing):
        label = f"{start}" if start == end else f"{start}-{end}"
        print(f"  {label}")
        if src:
            for n in range(start, min(end, len(src)) + 1):
                print(f"    {n:5}  {src[n - 1]}")
    return 0


# --- shapes -----------------------------------------------------------------

# A line is attributed to the first pattern that matches, so the order is the
# priority order: `unreachable!` before `panic!`, and both before the `?` that may
# sit on the same line.
SHAPE_PATTERNS = [
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


def cmd_shapes():
    args = positional()
    if not args:
        print(__doc__, file=sys.stderr)
        return 2
    report = args[0]
    skip = exceptions()

    counts = {name: 0 for name, _ in SHAPE_PATTERNS}
    counts["other"] = 0
    examples = {name: [] for name in counts}

    total = 0
    for entry in load(report)["files"]:
        name = entry["filename"]
        key = src_key(name)
        if key is None or key in skip:
            continue
        lines = entry["summary"]["lines"]
        if lines["count"] == 0 or lines["percent"] >= 98:
            continue

        seen, covered = region_entry_lines(entry)
        try:
            src = open(name).read().splitlines()
        except OSError:
            continue
        for n in sorted(seen - covered):
            if n > len(src):
                continue
            text = src[n - 1]
            total += 1
            for label, pattern in SHAPE_PATTERNS:
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
    return 0


# --- dead -------------------------------------------------------------------


def cmd_dead():
    args = positional()
    report = args[0] if args else DEFAULT_REPORT
    top = top_arg(30)
    skip = exceptions()

    total_count = defaultdict(int)
    span = {}
    label = {}
    for record in load(report)["functions"]:
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
        # The record's own (mangled) name, left as llvm-cov spells it: the fold is
        # by source position, so the name is only a pointer into the file.
        name = f"line {first}: {label[(filename, first)]}"
        short = src_key(filename)
        if short is None or short in skip:
            continue
        rows.append((span.get((filename, first), 0), short, name))

    rows.sort(reverse=True)
    print(f"{len(rows)} src/** functions never executed, spanning {sum(r[0] for r in rows)} lines")
    print(f"\n--- {top} widest")
    for width, short, name in rows[:top]:
        print(f"  {width:5} lines  {short}\n                {name}")
    return 0


# --- delta ------------------------------------------------------------------


def delta_rows(path):
    rows = {}
    for entry in load(path)["files"]:
        name = src_key(entry["filename"])
        if name is None:
            continue
        lines = entry["summary"]["lines"]
        if lines["count"] == 0:
            continue
        rows[name] = (lines["percent"], lines["covered"], lines["count"])
    return rows


def cmd_delta():
    args = positional()
    if not args:
        print(__doc__, file=sys.stderr)
        return 2
    limit = floor()
    skip = exceptions()

    def below(rows):
        return {n: v for n, v in rows.items() if v[0] < limit and n not in skip}

    base = delta_rows(args[0])
    if len(args) < 2:
        under = below(base)
        for name, (pct, cov, total) in sorted(under.items(), key=lambda kv: kv[1]):
            print(f"{pct:6.2f}%  ({cov}/{total})  {name}")
        print(f"\n{len(under)} of {len(base)} src/** files below {limit:.0f}%")
        return 0

    cur = delta_rows(args[1])
    moved = [
        (cur[n][0] - base[n][0], n, base[n], cur[n])
        for n in sorted(set(base) & set(cur))
        if abs(cur[n][0] - base[n][0]) > 0.005
    ]
    for change, name, b, c in sorted(moved):
        print(f"{change:+7.2f}  {b[0]:6.2f}% -> {c[0]:6.2f}%  ({c[1]}/{c[2]})  {name}")
    added = sorted(set(cur) - set(base))
    for name in added:
        print(f"    new  {cur[name][0]:6.2f}%  ({cur[name][1]}/{cur[name][2]})  {name}")
    print(
        f"\nbelow {limit:.0f}%: {len(below(base))} -> {len(below(cur))}"
        f"   ({len(moved)} files moved, {len(added)} new)"
    )
    return 0


COMMANDS = {
    "gaps": cmd_gaps,
    "lines": cmd_lines,
    "shapes": cmd_shapes,
    "dead": cmd_dead,
    "delta": cmd_delta,
}


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in COMMANDS:
        print(__doc__, file=sys.stderr)
        return 2
    return COMMANDS[sys.argv[1]]()


if __name__ == "__main__":
    sys.exit(main())
