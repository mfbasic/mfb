#!/usr/bin/env python3
"""Generate src/builtins/regex_unicode.mfb — the pinned Unicode general-category
table used by the regex package (see the embedded spec: `mfb spec stdlib regex`).

The table is emitted as MFBASIC source (one flat IF-chain function) rather than a
data table because MFBASIC list reads copy the whole list, and because the native
backends cannot hold a large constant array cheaply. The whole regex dialect's
Unicode behavior (\\d, \\w, \\s, \\b, \\p{gc}) is resolved through __regex_genCat.

The general categories come from the running interpreter's bundled
`unicodedata`, whose Unicode version is tied to the Python *minor* version
(3.12 → 15.0.0, 3.13 → 15.1.0, 3.14 → 16.0.0). So this script's output — and the
`REM Pinned Unicode version` header it records — is only reproducible under the
same Python that produced the checked-in artifact. The artifact is pinned to
**Unicode 16.0.0**, i.e. **Python 3.14.x**; `scripts/check-generated.sh` (and CI,
which pins `actions/setup-python` to 3.14) reproduce it there. Regenerate under
Python 3.14 after a Unicode bump — a different interpreter silently drifts the
table:

    python3.14 scripts/gen_regex_unicode.py > src/builtins/regex_unicode.mfb
"""
import sys
import unicodedata

MAX = 0x110000


def gc(cp):
    # Surrogates have no chr(); Unicode assigns them general category Cs.
    if 0xD800 <= cp <= 0xDFFF:
        return "Cs"
    return unicodedata.category(chr(cp))


def main():
    runs = []
    start = 0
    cur = gc(0)
    for cp in range(1, MAX):
        g = gc(cp)
        if g != cur:
            runs.append((start, cp - 1, cur))
            start = cp
            cur = g
    runs.append((start, MAX - 1, cur))

    out = []
    out.append("REM GENERATED FILE — do not edit by hand.")
    out.append("REM Source: scripts/gen_regex_unicode.py")
    out.append(f"REM Pinned Unicode version: {unicodedata.unidata_version}")
    out.append("REM Maps a Unicode scalar value to its two-letter general category.")
    out.append("REM Runs are contiguous and cover 0 .. 0x10FFFF (Cs = surrogate,")
    out.append("REM Cn = unassigned). See the embedded spec: mfb spec stdlib regex.")
    out.append("")
    out.append("FUNC __regex_genCat(cp AS Integer) AS String")
    for lo, hi, c in runs:
        out.append(f'  IF cp <= {hi} THEN RETURN "{c}"')
    out.append('  RETURN "Cn"')
    out.append("END FUNC")
    out.append("")
    sys.stdout.write("\n".join(out))
    sys.stderr.write(f"{len(runs)} runs, Unicode {unicodedata.unidata_version}\n")


if __name__ == "__main__":
    main()
