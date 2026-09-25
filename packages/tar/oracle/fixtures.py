#!/usr/bin/env python3
"""Generate the tar package's test fixtures, reproducibly.

Every fixture is produced by an implementation that is not ours -- Python's
`tarfile` and `/usr/bin/bsdtar` -- together with the entry metadata that
implementation reports, so the tests compare this reader against independent
readers rather than against its own earlier output.

The three tar dialects each get their own fixture, because the whole point of
the reader is that they agree: a 200-byte name is stored in a GNU `L` record in
GNU_FORMAT and in a PAX `x` record in PAX_FORMAT, and both must come back as the
same name.

Run from the repository root:

    python3 packages/tar/oracle/fixtures.py

It rewrites `packages/tar/src/test_fixtures.mfb` and refreshes the archives in
`packages/tar/oracle/corpus/`.
"""

import base64
import io
import os
import subprocess
import sys
import tarfile
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PKG = os.path.dirname(HERE)
CORPUS = os.path.join(HERE, "corpus")
OUT_MFB = os.path.join(PKG, "src", "test_fixtures.mfb")

# Fixed so regenerating produces identical bytes.
MTIME = 1789821000  # 2026-09-19 12:30:00 UTC

LONG_NAME = "long/" + ("n" * 195)          # 200 bytes, past the 100-byte field
LONG_LINK = "target/" + ("t" * 110)        # 117 bytes, past the 100-byte field


def member(name, kind=tarfile.REGTYPE, size=0, mode=0o644, link=""):
    info = tarfile.TarInfo(name)
    info.type = kind
    info.size = size
    info.mode = mode
    info.mtime = MTIME
    info.uid = 501
    info.gid = 20
    info.uname = "builder"
    info.gname = "staff"
    info.linkname = link
    return info


def build(fmt, entries):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=fmt) as t:
        for info, payload in entries:
            if payload is None:
                t.addfile(info)
            else:
                info.size = len(payload)
                t.addfile(info, io.BytesIO(payload))
    return buf.getvalue()


def build_ustar():
    """Plain ustar: a file, a directory, a file inside it."""
    return build(tarfile.USTAR_FORMAT, [
        (member("readme.txt"), b"a readme\n"),
        (member("docs/", kind=tarfile.DIRTYPE, mode=0o755), None),
        (member("docs/inner.txt"), b"inside docs\n"),
    ])


def build_gnu():
    """GNU: a long name and a long link target become `L` and `K` records."""
    return build(tarfile.GNU_FORMAT, [
        (member("short.txt"), b"short name\n"),
        (member(LONG_NAME), b"a very long name\n"),
        (member("link", kind=tarfile.SYMTYPE, link=LONG_LINK), None),
    ])


def build_pax():
    """PAX: the same long name, carried in an `x` extended header instead."""
    return build(tarfile.PAX_FORMAT, [
        (member("short.txt"), b"short name\n"),
        (member(LONG_NAME), b"a very long name\n"),
        (member("café/naïve.txt"), b"unicode name\n"),
    ])


def build_links():
    """Symlink and hardlink entries, which are listed but not readable."""
    return build(tarfile.USTAR_FORMAT, [
        (member("real.txt"), b"the real file\n"),
        (member("soft", kind=tarfile.SYMTYPE, link="real.txt"), None),
        (member("hard", kind=tarfile.LNKTYPE, link="real.txt"), None),
    ])


def build_bsdtar():
    """An archive written by bsdtar, a third independent writer."""
    with tempfile.TemporaryDirectory() as tmp:
        src = os.path.join(tmp, "data")
        os.makedirs(src)
        with open(os.path.join(src, "one.txt"), "wb") as fh:
            fh.write(b"written by bsdtar\n")
        with open(os.path.join(src, "two.txt"), "wb") as fh:
            fh.write(b"xy" * 300)
        out = os.path.join(tmp, "made.tar")
        subprocess.run(
            ["/usr/bin/bsdtar", "-cf", out, "data"], cwd=tmp, check=True,
        )
        with open(out, "rb") as fh:
            return fh.read()


BUILDERS = [
    ("ustar", build_ustar),
    ("gnu", build_gnu),
    ("pax", build_pax),
    ("links", build_links),
    ("bsdtar", build_bsdtar),
]

KIND = {
    tarfile.REGTYPE: 0,
    tarfile.AREGTYPE: 0,
    tarfile.DIRTYPE: 1,
    tarfile.SYMTYPE: 2,
    tarfile.LNKTYPE: 3,
}


def describe(data):
    entries = []
    with tarfile.open(fileobj=io.BytesIO(data)) as t:
        for info in t.getmembers():
            content = b""
            if info.isreg():
                fh = t.extractfile(info)
                if fh is not None:
                    content = fh.read()
            name = info.name
            # tarfile strips a directory's trailing slash; the bytes keep it,
            # and so does this reader.
            if info.isdir() and not name.endswith("/"):
                name = name + "/"
            entries.append({
                "name": name,
                "kind": KIND.get(info.type, 4),
                "is_dir": info.isdir(),
                "size": info.size,
                "mode": info.mode,
                "mtime": int(info.mtime),
                "uid": info.uid,
                "gid": info.gid,
                "user": info.uname or "",
                "group": info.gname or "",
                "link": info.linkname or "",
                "content": content,
            })
    return entries


def mfb_string(text):
    out = []
    for ch in text:
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\r":
            out.append("\\r")
        elif ch == "\t":
            out.append("\\t")
        elif ord(ch) < 0x20 or ord(ch) == 0x7F:
            out.append("\\u%04X" % ord(ch))
        else:
            out.append(ch)
    return '"' + "".join(out) + '"'


HEADER = '''\' ---------------------------------------------------------------------------
\' GENERATED by packages/tar/oracle/fixtures.py -- do not edit by hand.
\'
\' Each fixture is an archive written by an implementation that is not ours
\' (Python `tarfile` in each of its three formats, or `/usr/bin/bsdtar`),
\' together with the entry metadata that implementation reports for it. A
\' disagreement in the tests is a disagreement with an independent reader rather
\' than with our own earlier output.
\'
\' The `gnu` and `pax` fixtures carry the SAME 200-byte name, stored two
\' different ways -- a GNU `L` record and a PAX `x` record. Both must read back
\' as the same name; that is the point of the reader.
\'
\' Archive bytes are Base64 rather than byte-list literals: these total tens of
\' kilobytes and a `[toByte(...), ...]` literal of that size is tens of
\' thousands of calls to compile for no added clarity.
\'
\' Regenerate with:  python3 packages/tar/oracle/fixtures.py
\' ---------------------------------------------------------------------------

IMPORT encoding

\' What the oracle says one entry of a fixture contains.
EXPORT TYPE ExpectedEntry
  name AS String
  kind AS Integer
  isDirectory AS Boolean
  size AS Integer
  mode AS Integer
  modifiedSeconds AS Integer
  uid AS Integer
  gid AS Integer
  user AS String
  group AS String
  linkTarget AS String
  contentBase64 AS String
END TYPE

\' One archive, plus what the oracle says is in it.
EXPORT TYPE Fixture
  label AS String
  bytes AS List OF Byte
  entries AS List OF ExpectedEntry
END TYPE

\' The 200-byte name the `gnu` and `pax` fixtures share.
EXPORT FUNC longName() AS String
  RETURN %s
END FUNC

\' The 117-byte link target the `gnu` fixture uses.
EXPORT FUNC longLink() AS String
  RETURN %s
END FUNC

'''


def emit():
    os.makedirs(CORPUS, exist_ok=True)
    chunks = []
    labels = []
    for label, build_fn in BUILDERS:
        data = build_fn()
        with open(os.path.join(CORPUS, label + ".tar"), "wb") as fh:
            fh.write(data)
        entries = describe(data)
        labels.append(label)
        lines = ["PRIVATE FUNC fixture_%s() AS Fixture" % label]
        lines.append("  RETURN Fixture[%s, _" % mfb_string(label))
        lines.append("    encoding::base64Decode(%s), _"
                     % mfb_string(base64.b64encode(data).decode()))
        if entries:
            lines.append("    [ _")
            rows = []
            for e in entries:
                rows.append(
                    "      ExpectedEntry[%s, %d, %s, %d, %d, %d, %d, %d, %s, %s, %s, %s]"
                    % (
                        mfb_string(e["name"]), e["kind"],
                        "TRUE" if e["is_dir"] else "FALSE",
                        e["size"], e["mode"], e["mtime"], e["uid"], e["gid"],
                        mfb_string(e["user"]), mfb_string(e["group"]),
                        mfb_string(e["link"]),
                        mfb_string(base64.b64encode(e["content"]).decode()),
                    )
                )
            lines.append(", _\n".join(rows) + " _")
            lines.append("    ]]")
        else:
            lines.append("    []]")
        lines.append("END FUNC")
        chunks.append("\n".join(lines))

    listing = ("EXPORT FUNC allFixtures() AS List OF Fixture\n  RETURN [ _\n"
               + ", _\n".join("    fixture_%s()" % l for l in labels)
               + " _\n  ]\nEND FUNC\n")
    with open(OUT_MFB, "w") as fh:
        fh.write(HEADER % (mfb_string(LONG_NAME), mfb_string(LONG_LINK))
                 + "\n\n".join(chunks) + "\n\n" + listing)
    print("wrote %s (%d fixtures)" % (OUT_MFB, len(labels)))
    for label in labels:
        print("  corpus/%s.tar" % label)


if __name__ == "__main__":
    sys.exit(emit())
