#!/usr/bin/env python3
"""Generate src/codegen/string/unicode/unicode_gencat_ranges.txt — the pinned
Unicode general-category table as DATA, for the native rodata lookup that
replaces the generated `__regex_genCat` / `__strings_genCat` IF-chains
(plan-118-B).

Same runs, same pinned Unicode version, same interpreter requirement as
`gen_regex_unicode.py` — this script imports that generator's `runs()` rather
than recomputing the categories, so the two artifacts cannot disagree about a
scalar. The artifact is pinned to **Unicode 16.0.0**, i.e. **Python 3.14.x**;
`scripts/check-generated.sh` reproduces it there.

    python3.14 scripts/gen_unicode_gencat_table.py > src/codegen/string/unicode/unicode_gencat_ranges.txt

Why this table is NOT the vendored utf8proc property trie, which already ships a
general-category field: measured 2026-09-01, utf8proc 2.11.3's categories
disagree with pinned Unicode 16.0.0 on **4,804 scalars** (4,803 of them `Cn` here
and assigned there — utf8proc carries a newer UCD — plus U+0295 `Ll` vs `Lo`).
Worse, 19 of its 8,385 deduplicated property rows are shared by scalars whose
16.0.0 categories differ, so a category field packed into that record cannot
even represent these answers. The trie's row identity is utf8proc's, not ours.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import unicodedata  # noqa: E402

from gen_regex_unicode import runs  # noqa: E402


def main():
    table = runs()
    categories = sorted({category for _lo, _hi, category in table})

    out = []
    out.append("# GENERATED FILE — do not edit by hand.")
    out.append("# Source: scripts/gen_unicode_gencat_table.py")
    out.append(f"# Pinned Unicode version: {unicodedata.unidata_version}")
    out.append("# The same runs as unicode_gencat.mfb, as data: one line per run,")
    out.append("# `<last codepoint of the run, decimal> <two-letter category>`, in")
    out.append("# ascending order, contiguous, covering 0 .. 0x10FFFF (Cs = surrogate,")
    out.append("# Cn = unassigned). A lookup binary-searches for the first line whose")
    out.append("# codepoint is >= the query.")
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
