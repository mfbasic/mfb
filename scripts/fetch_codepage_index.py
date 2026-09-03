#!/usr/bin/env python3
"""Re-fetch the WHATWG legacy single-byte index files into tools/codepage-index/.

The files are vendored so the build is network-free and the generated tables are
auditable by `diff` against upstream. Run this only to refresh them; then `git diff`
tools/codepage-index/ to review what upstream changed, and re-run
`scripts/gen-codepage-tables.py`.

`ISO-8859-8-I` has no index file of its own (HTTP 404) and shares `ISO-8859-8`'s
table; it is reported as `shares ISO-8859-8` rather than as a failure.
"""

import json
import os
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "tools", "codepage-index")
ENCODINGS_JSON = "https://encoding.spec.whatwg.org/encodings.json"
INDEX_URL = "https://encoding.spec.whatwg.org/index-{}.txt"


def main() -> int:
    os.makedirs(OUT, exist_ok=True)
    with urllib.request.urlopen(ENCODINGS_JSON, timeout=60) as r:
        groups = json.load(r)

    names = []
    for group in groups:
        if "single-byte" in group["heading"]:
            names = [e["name"] for e in group["encodings"]]
    if not names:
        print("no single-byte group in encodings.json", file=sys.stderr)
        return 1

    written = 0
    for name in names:
        label = name.lower()
        try:
            with urllib.request.urlopen(INDEX_URL.format(label), timeout=60) as r:
                data = r.read()
        except urllib.error.HTTPError as exc:
            if exc.code == 404 and label == "iso-8859-8-i":
                print(f"{name:16s} shares ISO-8859-8 (no index of its own)")
                continue
            print(f"{name:16s} FAILED {exc}", file=sys.stderr)
            return 1
        with open(os.path.join(OUT, f"index-{label}.txt"), "wb") as f:
            f.write(data)
        written += 1
        print(f"{name:16s} ok")

    print(f"labels={len(names)} files={written}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
