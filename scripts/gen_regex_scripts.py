#!/usr/bin/env python3
"""Generate src/codegen/string/unicode/unicode_script_names.mfb — the canonical
spelling of each Unicode Script name, for the regex engine's `\\p{Script=...}`
(see `mfb spec stdlib regex`).

  * `__regex_scriptCanonName(low)` — the canonical name for a lowercased script
                                     name, or "" when it is not a script.

It also exposes `runs()` for `gen_unicode_script_table.py`, which emits the
per-SCALAR half of the property (`unicode_script_ranges.txt`) as rodata. That
half used to be generated here too, as `__regex_scriptOf` — a 1,708-arm
MFBASIC IF-chain compiled into every program at 440,905 machine instructions and
scanned linearly per query (plan-118-B). One `runs()`, two artifacts, so they
cannot disagree about a scalar. This 171-arm name table stays MFBASIC: it is
looked up once per pattern compile, by name, not per scalar.

Python's `unicodedata` exposes NO Script property, so the data comes from a
VENDORED copy of the Unicode Character Database `Scripts.txt`, pinned to Unicode
16.0.0 (`third_party/unicode/Scripts-16.0.0.txt`). Because the generator reads
only that committed file — never the network or the interpreter's Unicode tables
— its output is reproducible under any Python 3, so
`scripts/check-generated.sh` verifies it the same way it verifies the other
generated artifacts.

    python3 scripts/gen_regex_scripts.py > src/codegen/string/unicode/unicode_script_names.mfb
"""
import os
import sys

MAX = 0x110000
SCRIPTS_TXT = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    "..",
    "third_party",
    "unicode",
    "Scripts-16.0.0.txt",
)


def parse_scripts():
    scripts = ["Unknown"] * MAX
    version = None
    with open(SCRIPTS_TXT, encoding="utf-8") as handle:
        for raw in handle:
            if version is None and raw.startswith("# Scripts-"):
                # "# Scripts-16.0.0.txt" -> "16.0.0"
                version = raw.strip()[len("# Scripts-") : -len(".txt")]
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            rng, name = (part.strip() for part in line.split(";"))
            if ".." in rng:
                lo_hex, hi_hex = rng.split("..")
            else:
                lo_hex = hi_hex = rng
            lo, hi = int(lo_hex, 16), int(hi_hex, 16)
            for cp in range(lo, hi + 1):
                scripts[cp] = name
    return scripts, version


def runs():
    """`(runs, names, version)` — contiguous (lo, hi, script) runs covering
    0 .. 0x10FFFF, the sorted non-`Unknown` script names, and the pinned Unicode
    version.

    Shared with `gen_unicode_script_table.py`, which emits the same runs as a
    native rodata range table (plan-118-B). One computation, two artifacts, so
    they cannot disagree about a scalar.
    """
    scripts, version = parse_scripts()
    out = []
    start = 0
    cur = scripts[0]
    for cp in range(1, MAX):
        if scripts[cp] != cur:
            out.append((start, cp - 1, cur))
            start = cp
            cur = scripts[cp]
    out.append((start, MAX - 1, cur))
    names = sorted(name for name in set(scripts) if name != "Unknown")
    return out, names, version


def main():
    _runs, names, version = runs()

    out = []
    out.append("REM GENERATED FILE — do not edit by hand.")
    out.append("REM Source: scripts/gen_regex_scripts.py")
    out.append(f"REM Pinned Unicode version: {version}")
    out.append("REM Data: third_party/unicode/Scripts-16.0.0.txt (UCD Script property).")
    out.append("REM Maps a lowercased script name to its canonical spelling, for `\\p{Script=…}`.")
    out.append("REM The per-scalar table that used to head this file is now the rodata run table")
    out.append("REM `unicode_script_ranges.txt`, looked up by `regex::scriptOf` (plan-118-B).")
    out.append("")
    out.append("FUNC __regex_scriptCanonName(low AS String) AS String")
    for name in names:
        out.append(f'  IF low = "{name.lower()}" THEN RETURN "{name}"')
    out.append('  RETURN ""')
    out.append("END FUNC")
    out.append("")

    sys.stdout.write("\n".join(out))
    sys.stderr.write(f"{len(names)} scripts, Unicode {version}\n")


if __name__ == "__main__":
    main()
