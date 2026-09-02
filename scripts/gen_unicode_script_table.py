#!/usr/bin/env python3
"""Generate src/codegen/string/unicode/unicode_script_ranges.txt — the pinned
Unicode Script property as DATA, for the native rodata lookup that replaces the
generated `__regex_scriptOf` IF-chain (plan-118-B).

Imports `gen_regex_scripts.runs()` rather than re-reading the UCD, so this
artifact and the script NAME table cannot disagree about a scalar. The data is
the vendored `third_party/unicode/Scripts-16.0.0.txt`, never the network or the
interpreter's tables, so the output is reproducible under any Python 3 —
`scripts/check-generated.sh` verifies it the same way it verifies the .mfb.

    python3 scripts/gen_unicode_script_table.py > src/codegen/string/unicode/unicode_script_ranges.txt
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from gen_regex_scripts import runs  # noqa: E402


def main():
    table, names, version = runs()
    # `Unknown` is the no-script answer and is not in `names`; it is a script
    # name the lookup can return, so it has to be in the emitted name set.
    every = sorted(set(names) | {"Unknown"})

    out = []
    out.append("# GENERATED FILE — do not edit by hand.")
    out.append("# Source: scripts/gen_unicode_script_table.py")
    out.append(f"# Pinned Unicode version: {version}")
    out.append("# Data: third_party/unicode/Scripts-16.0.0.txt (UCD Script property).")
    out.append("# One line per run, ascending and contiguous over 0 .. 0x10FFFF:")
    out.append("# `<last codepoint of the run, decimal> <canonical script name>`")
    out.append("# (Unknown = no script). A lookup binary-searches for the first line")
    out.append("# whose codepoint is >= the query.")
    out.append(f"# scripts: {len(every)}")
    for _lo, hi, name in table:
        out.append(f"{hi} {name}")
    out.append("")
    sys.stdout.write("\n".join(out))
    sys.stderr.write(f"{len(table)} runs, {len(every)} scripts, Unicode {version}\n")


if __name__ == "__main__":
    main()
