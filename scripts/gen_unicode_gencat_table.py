#!/usr/bin/env python3
"""Generate src/codegen/string/unicode/unicode_gencat_ranges.txt — the pinned
Unicode general-category table, read as rodata by `regex::genCat` /
`strings::genCat` (plan-118-B).

Replaces `gen_regex_unicode.py`, which emitted the same runs as one flat
MFBASIC IF-chain function per package -- 4,099 `IF cp <= N THEN RETURN "Lu"`
arms, compiled into every program at 1,057,783 machine instructions per copy,
and a linear scan of up to 4,099 compares per query.

The general categories come from the running interpreter's bundled
`unicodedata`, whose Unicode version is tied to the Python *minor* version
(3.12 -> 15.0.0, 3.13 -> 15.1.0, 3.14 -> 16.0.0). So this script's output -- and
the `Pinned Unicode version` header it records -- is only reproducible under the
same Python that produced the checked-in artifact. The artifact is pinned to
**Unicode 16.0.0**, i.e. **Python 3.14.x**; `scripts/check-generated.sh` (and CI,
which pins `actions/setup-python` to 3.14) reproduce it there. Regenerate under
Python 3.14 after a Unicode bump -- a different interpreter silently drifts the
table:

    python3.14 scripts/gen_unicode_gencat_table.py > src/codegen/string/unicode/unicode_gencat_ranges.txt

Why this table is NOT the vendored utf8proc property trie, which already ships a
general-category field: measured 2026-09-01, utf8proc 2.11.3's categories
disagree with pinned Unicode 16.0.0 on **4,804 scalars** (4,803 of them `Cn` here
and assigned there — utf8proc carries a newer UCD — plus U+0295 `Ll` vs `Lo`).
Worse, 19 of its 8,385 deduplicated property rows are shared by scalars whose
16.0.0 categories differ, so a category field packed into that record cannot
even represent these answers. The trie's row identity is utf8proc's, not ours.
"""
import sys
import unicodedata

MAX = 0x110000


def gc(cp):
    # Surrogates have no chr(); Unicode assigns them general category Cs.
    if 0xD800 <= cp <= 0xDFFF:
        return "Cs"
    return unicodedata.category(chr(cp))


def runs():
    """The contiguous (lo, hi, category) runs covering 0 .. 0x10FFFF."""
    out = []
    start = 0
    cur = gc(0)
    for cp in range(1, MAX):
        g = gc(cp)
        if g != cur:
            out.append((start, cp - 1, cur))
            start = cp
            cur = g
    out.append((start, MAX - 1, cur))
    return out


def main():
    table = runs()
    categories = sorted({category for _lo, _hi, category in table})

    out = []
    out.append("# GENERATED FILE — do not edit by hand.")
    out.append("# Source: scripts/gen_unicode_gencat_table.py")
    out.append(f"# Pinned Unicode version: {unicodedata.unidata_version}")
    out.append("# One line per run, ascending and contiguous over 0 .. 0x10FFFF:")
    out.append("# `<last codepoint of the run, decimal> <two-letter category>`")
    out.append("# (Cs = surrogate, Cn = unassigned). A lookup binary-searches for the")
    out.append("# first line whose codepoint is >= the query.")
    out.append(f"# categories: {' '.join(categories)}")
    for _lo, hi, category in table:
        out.append(f"{hi} {category}")
    out.append("")
    sys.stdout.write("\n".join(out))
    sys.stderr.write(
        f"{len(table)} runs, {len(categories)} categories, "
        f"Unicode {unicodedata.unidata_version}\n"
    )


if __name__ == "__main__":
    main()
