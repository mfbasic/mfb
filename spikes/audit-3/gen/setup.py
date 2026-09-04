"""Materialise the DEC-* spike projects under /tmp/audit-3-spikes."""
import json, os

BASE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

TMPL = {
    "name": None, "version": "0.1.0", "mfb": "1.0", "kind": "executable",
    "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}],
    "entry": "main", "targets": ["native"],
}

for d in ["DEC-50", "DEC-51", "DEC-52", "DEC-53", "DEC-54", "DEC-55"]:
    p = dict(TMPL)
    p["name"] = d
    os.makedirs(os.path.join(BASE, d, "src"), exist_ok=True)
    with open(os.path.join(BASE, d, "project.json"), "w") as f:
        f.write(json.dumps(p, indent=2) + "\n")

HEADERS = {
    "DEC-51": ("""' DEC-51 -- __canvas_inflate has no cap on decompressed output (zlib bomb), and the
' PNG decode then reports SUCCESS.
'
' Craft the input with:  python3 gen/mkbomb.py 400000000 /tmp/dec51.png
'   (389 KB: a 1x1 IHDR whose IDAT is zlib(400 MB of zeros))
'
' Observed: 25.0 GB maximum resident set size, 24 s, exit 0, prints "decoded 1x1".
' Expected: refused once the inflated output passes what a 1x1 image can need.
""", "/tmp/dec51.png"),
    "DEC-52": ("""' DEC-52 -- __canvas_pngSlice copies the accumulator per chunk, so multi-chunk IDAT
' accumulation is quadratic AND leaks every intermediate copy into the arena.
'
' Craft the input with:  python3 gen/mkchunks.py 20000 8 /tmp/dec52.png
'   (400 KB file carrying only 160 KB of IDAT payload, split into 20000 chunks)
'
' Observed: 2.47 GB maximum resident set size, 5.6 s, and it succeeds.
' Expected: cost linear in the file size (~400 KB of work).
""", "/tmp/dec52.png"),
}

src50 = open(os.path.join(BASE, "DEC-50/src/main.mfb")).read()
body = "IMPORT app" + src50.split("IMPORT app", 1)[1]
for d, (hdr, default) in HEADERS.items():
    out = hdr + body.replace("/tmp/dec50.png", default)
    with open(os.path.join(BASE, d, "src/main.mfb"), "w") as f:
        f.write(out)

print("wrote project.json x6 and DEC-51/DEC-52 sources")
