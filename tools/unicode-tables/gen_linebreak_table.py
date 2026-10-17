#!/usr/bin/env python3
"""Generate src/codegen/string/unicode/unicode_linebreak_ranges.txt — the Unicode
Line_Break property, resolved for UAX #14 (plan-162-E), as DATA for the native
rodata lookup `strings::lineBreakClass`.

Every input is a vendored, pinned file, never the interpreter's tables, so the output
is reproducible under any Python 3 and `scripts/check-generated.sh` verifies it:

* `third_party/unicode/LineBreak-16.0.0.txt` — Line_Break;
* `third_party/unicode/EastAsianWidth-16.0.0.txt` — East_Asian_Width (LB19a, LB21a,
  LB30 exclude Fullwidth, Wide and Halfwidth);
* `third_party/unicode/emoji-data-16.0.0.txt` — Extended_Pictographic (LB30b);
* `src/codegen/string/unicode/unicode_gencat_ranges.txt` — General_Category, the
  committed artifact of `gen_unicode_gencat_table.py` (LB1's SA split, LB30b's Cn,
  and LB15a/LB15b/LB19's Pi and Pf quotation marks).

LB1's default resolutions are applied here, so the lookup answers a class the
algorithm uses directly: AI, SG and XX are AL; SA is CM for Mn or Mc and AL otherwise;
CJ is NS. CB is kept (LB20 breaks around it).

Each value is `class + 64 * eastAsian + 128 * pictographicUnassigned + 256 * Pi +
512 * Pf`, with the class numbered as `CLASSES` lists them. Pi and Pf are recorded on
QU only, the one class whose rules read them, which keeps the run count down.

    python3 tools/unicode-tables/gen_linebreak_table.py > src/codegen/string/unicode/unicode_linebreak_ranges.txt
"""
import os
import re
import sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
UCD = os.path.join(ROOT, "third_party", "unicode")
GENCAT = os.path.join(ROOT, "src", "codegen", "string", "unicode", "unicode_gencat_ranges.txt")

# The resolved classes, in the order their numbers are assigned. The MFBASIC
# algorithm (`func_line_breaks.rs`) names the same numbers; keep the two in step.
CLASSES = [
    "BK", "CR", "LF", "NL", "SP", "ZW", "ZWJ", "CM", "WJ", "GL", "BA", "BB", "B2",
    "HY", "CB", "CL", "CP", "EX", "IN", "NS", "OP", "QU", "IS", "NU", "PO", "PR",
    "SY", "AL", "HL", "ID", "EB", "EM", "H2", "H3", "JL", "JV", "JT", "RI", "AK",
    "AP", "AS", "VF", "VI",
]
EAST_ASIAN = 64
PICTOGRAPHIC_UNASSIGNED = 128
PI = 256
PF = 512
MAX = 0x10FFFF

LINE = re.compile(r"^([0-9A-F]{4,6})(?:\.\.([0-9A-F]{4,6}))?\s*;\s*([A-Za-z0-9_]+)")


def ucd_property(name, default):
    """A per-codepoint list from a UCD property file whose @missing is `default`."""
    values = [default] * (MAX + 1)
    with open(os.path.join(UCD, name), encoding="utf-8") as f:
        for raw in f:
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            m = LINE.match(line)
            if not m:
                raise SystemExit(f"{name}: unparsed line {raw!r}")
            lo = int(m.group(1), 16)
            hi = int(m.group(2), 16) if m.group(2) else lo
            for cp in range(lo, hi + 1):
                values[cp] = m.group(3)
    return values


def missing_default(name):
    with open(os.path.join(UCD, name), encoding="utf-8") as f:
        for raw in f:
            if raw.startswith("# @missing: 0000..10FFFF;"):
                return raw.split(";", 1)[1].strip()
    raise SystemExit(f"{name}: no whole-range @missing line")


def pictographic():
    flags = [False] * (MAX + 1)
    with open(os.path.join(UCD, "emoji-data-16.0.0.txt"), encoding="utf-8") as f:
        for raw in f:
            line = raw.split("#", 1)[0].strip()
            m = LINE.match(line) if line else None
            if m and m.group(3) == "Extended_Pictographic":
                lo = int(m.group(1), 16)
                hi = int(m.group(2), 16) if m.group(2) else lo
                for cp in range(lo, hi + 1):
                    flags[cp] = True
    return flags


def gencat():
    cats = [None] * (MAX + 1)
    start = 0
    with open(GENCAT, encoding="utf-8") as f:
        for raw in f:
            if raw.startswith("#") or not raw.strip():
                continue
            end, cat = raw.split()
            for cp in range(start, int(end) + 1):
                cats[cp] = cat
            start = int(end) + 1
    if start != MAX + 1:
        raise SystemExit("unicode_gencat_ranges.txt does not cover 0..0x10FFFF")
    return cats


def resolve(cls, cat):
    if cls in ("AI", "SG", "XX"):
        return "AL"
    if cls == "SA":
        return "CM" if cat in ("Mn", "Mc") else "AL"
    if cls == "CJ":
        return "NS"
    return cls


def main():
    lb = ucd_property("LineBreak-16.0.0.txt", missing_default("LineBreak-16.0.0.txt"))
    ea = ucd_property("EastAsianWidth-16.0.0.txt", missing_default("EastAsianWidth-16.0.0.txt"))
    ep = pictographic()
    cats = gencat()

    runs = []
    for cp in range(MAX + 1):
        cls = resolve(lb[cp], cats[cp])
        if cls not in CLASSES:
            raise SystemExit(f"U+{cp:04X}: unexpected class {cls}")
        value = CLASSES.index(cls)
        if ea[cp] in ("F", "W", "H"):
            value += EAST_ASIAN
        if ep[cp] and cats[cp] == "Cn":
            value += PICTOGRAPHIC_UNASSIGNED
        if cls == "QU" and cats[cp] == "Pi":
            value += PI
        if cls == "QU" and cats[cp] == "Pf":
            value += PF
        if runs and runs[-1][1] == value:
            runs[-1][0] = cp
        else:
            runs.append([cp, value])

    out = []
    out.append("# GENERATED FILE — do not edit by hand.")
    out.append("# Source: tools/unicode-tables/gen_linebreak_table.py")
    out.append("# Pinned Unicode version: 16.0.0")
    out.append("# Data: third_party/unicode/LineBreak-16.0.0.txt, EastAsianWidth-16.0.0.txt,")
    out.append("# emoji-data-16.0.0.txt, and src/codegen/string/unicode/unicode_gencat_ranges.txt.")
    out.append("# One line per run, ascending and contiguous over 0 .. 0x10FFFF:")
    out.append("# `<last codepoint of the run, decimal> <value>`, where value is the UAX #14")
    out.append("# class after LB1's resolutions (numbered in this order:")
    out.append("# " + " ".join(CLASSES) + ")")
    out.append("# + 64 East_Asian_Width F/W/H + 128 Extended_Pictographic & Cn")
    out.append("# + 256 Pi quotation mark + 512 Pf quotation mark (QU only).")
    out.append(f"# runs: {len(runs)}")
    for end, value in runs:
        out.append(f"{end} {value}")
    out.append("")
    sys.stdout.write("\n".join(out))
    sys.stderr.write(f"{len(runs)} runs, Unicode 16.0.0\n")


if __name__ == "__main__":
    main()
