#!/usr/bin/env python3
"""Generate src/unicode/casefold_cf.txt — the Unicode full case folding (the C and
F mappings of CaseFolding.txt) behind `strings::caseFold` and the regex `(?i)` flag.

The only input is the vendored, pinned `third_party/unicode/CaseFolding-16.0.0.txt`
(the Unicode version every other pinned table uses), so the output is reproducible
under any Python 3 and `scripts/check-generated.sh` verifies it. S (simple) and T
(Turkic) mappings are not part of full case folding and are skipped.

One line per folded scalar, sorted: `<hex scalar>;<hex scalar> [<hex scalar>...]`.
Both the compile-time fold (`src/unicode/backend.rs:case_fold`) and the runtime
casefold tables (`src/unicode/runtime_tables.rs`) read it through `src/unicode/casefold.rs`.

    python3 tools/unicode-tables/gen_casefold_table.py > src/unicode/casefold_cf.txt
"""
import os

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
SOURCE = os.path.join(ROOT, "third_party", "unicode", "CaseFolding-16.0.0.txt")


def mappings():
    out = {}
    with open(SOURCE, encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            code, status, target, _ = [p.strip() for p in line.split(";")]
            if status not in ("C", "F"):
                continue
            cp = int(code, 16)
            assert cp not in out, f"duplicate fold for {code}"
            out[cp] = [int(t, 16) for t in target.split()]
    return out


def main():
    print("# GENERATED FILE - do not edit by hand.")
    print("# Source: tools/unicode-tables/gen_casefold_table.py")
    print("# Pinned Unicode version: 16.0.0")
    print("# Data: third_party/unicode/CaseFolding-16.0.0.txt (status C and F).")
    for cp, target in sorted(mappings().items()):
        print("%04X;%s" % (cp, " ".join("%04X" % t for t in target)))


if __name__ == "__main__":
    main()
