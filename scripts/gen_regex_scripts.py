#!/usr/bin/env python3
"""Generate src/codegen/unicode/unicode_script_of.mfb — the full Unicode Script property
table used by the regex engine's `\\p{Script=...}` (see `mfb spec stdlib regex`).

The 10 hand-written script ranges the engine shipped with are replaced by two
generated functions:

  * `__regex_scriptOf(cp)`        — the canonical Script name for a scalar, or
                                    "Unknown" for scalars with no assigned script.
  * `__regex_scriptCanonName(low) — the canonical name for a lowercased script
                                    name, or "" when it is not a script.

Emitted as MFBASIC source (flat IF-chains) for the same reason as the
general-category table (`gen_regex_unicode.py`): MFBASIC list reads copy the
whole list and the native backends cannot hold a large constant array cheaply.

Unlike the general-category table, Python's `unicodedata` exposes NO Script
property, so the data comes from a VENDORED copy of the Unicode Character
Database `Scripts.txt`, pinned to Unicode 16.0.0
(`third_party/unicode/Scripts-16.0.0.txt`). Because the generator reads only that
committed file — never the network or the interpreter's Unicode tables — its
output is reproducible under any Python 3, so `scripts/check-generated.sh`
verifies it the same way it verifies the other generated artifacts.

    python3 scripts/gen_regex_scripts.py > src/codegen/unicode/unicode_script_of.mfb
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


def main():
    scripts, version = parse_scripts()

    runs = []
    start = 0
    cur = scripts[0]
    for cp in range(1, MAX):
        if scripts[cp] != cur:
            runs.append((start, cp - 1, cur))
            start = cp
            cur = scripts[cp]
    runs.append((start, MAX - 1, cur))

    names = sorted(name for name in set(scripts) if name != "Unknown")

    out = []
    out.append("REM GENERATED FILE — do not edit by hand.")
    out.append("REM Source: scripts/gen_regex_scripts.py")
    out.append(f"REM Pinned Unicode version: {version}")
    out.append("REM Data: third_party/unicode/Scripts-16.0.0.txt (UCD Script property).")
    out.append("REM Runs are contiguous and cover 0 .. 0x10FFFF (Unknown = no script).")
    out.append("")
    out.append("FUNC __regex_scriptOf(cp AS Integer) AS String")
    for _lo, hi, name in runs:
        out.append(f'  IF cp <= {hi} THEN RETURN "{name}"')
    out.append('  RETURN "Unknown"')
    out.append("END FUNC")
    out.append("")
    out.append("FUNC __regex_scriptCanonName(low AS String) AS String")
    for name in names:
        out.append(f'  IF low = "{name.lower()}" THEN RETURN "{name}"')
    out.append('  RETURN ""')
    out.append("END FUNC")
    out.append("")

    sys.stdout.write("\n".join(out))
    sys.stderr.write(
        f"{len(runs)} runs, {len(names)} scripts, Unicode {version}\n"
    )


if __name__ == "__main__":
    main()
