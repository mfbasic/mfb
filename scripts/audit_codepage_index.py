#!/usr/bin/env python3
"""Audit the vendored WHATWG index files (plan-123).

Reports, per file, the mapping count, the highest code point, and whether any code
point repeats within the file. Three plan-123 design premises rest on these numbers:

  * the highest code point across all files is below U+FFFD, so U+FFFD is an
    unambiguous "byte unmapped" sentinel in a table literal;
  * every mapping is a single BMP scalar, so one 128-scalar String literal holds a
    whole table;
  * no code point repeats within a file, so `codepageEncode`'s reverse lookup by
    scalar search cannot pick the wrong byte.

Exits non-zero if any premise fails.
"""

import glob
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INDEX_DIR = os.path.join(ROOT, "tools", "codepage-index")
SENTINEL = 0xFFFD


def read_index(path):
    """Return {pointer: codepoint} for one index file."""
    rows = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            text = line.split("#")[0].strip()
            if not text:
                continue
            fields = text.split()
            rows[int(fields[0])] = int(fields[1], 16)
    return rows


def main() -> int:
    paths = sorted(glob.glob(os.path.join(INDEX_DIR, "index-*.txt")))
    if not paths:
        print(f"no index files under {INDEX_DIR}", file=sys.stderr)
        return 1

    total = 0
    overall_max = 0
    overall_max_file = ""
    dup_files = []
    for path in paths:
        rows = read_index(path)
        by_cp = {}
        dups = []
        for ptr, cp in sorted(rows.items()):
            if cp in by_cp:
                dups.append((cp, by_cp[cp], ptr))
            else:
                by_cp[cp] = ptr
        name = os.path.basename(path)
        top = max(rows.values())
        total += len(rows)
        if top > overall_max:
            overall_max, overall_max_file = top, name
        if dups:
            dup_files.append((name, dups))
        print(f"{name:26s} mappings={len(rows):3d} max=U+{top:04X} dups={len(dups)}")

    print()
    print(f"files: {len(paths)}")
    print(f"total mappings: {total}")
    print(f"max code point: U+{overall_max:04X} in {overall_max_file}")
    print(f"files with a repeated code point: {len(dup_files)}")

    ok = True
    if overall_max >= SENTINEL:
        print(
            f"FAIL: U+{SENTINEL:04X} is not a safe hole sentinel "
            f"(a table maps U+{overall_max:04X})",
            file=sys.stderr,
        )
        ok = False
    if overall_max > 0xFFFF:
        print("FAIL: a mapping is outside the BMP", file=sys.stderr)
        ok = False
    for name, dups in dup_files:
        print(f"FAIL: {name} repeats {len(dups)} code point(s): {dups[:5]}", file=sys.stderr)
        ok = False
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
