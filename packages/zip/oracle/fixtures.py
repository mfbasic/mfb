#!/usr/bin/env python3
"""Generate the zip package's test fixtures, reproducibly.

Every fixture in `packages/zip/src/test_fixtures.mfb` is produced here, by an
implementation that is not ours -- Python's `zipfile` and `/usr/bin/zip` -- so
the tests check our reader against somebody else's writer rather than against
itself. The expected entry metadata is read back out of `zipfile` too, so the
numbers the tests assert are the oracle's numbers, not ours.

One fixture (`cp437`) is built by hand from `struct.pack`, because `zipfile`
cannot be made to write a non-UTF-8 name with general-purpose flag bit 11 clear
-- it always encodes a non-ASCII name as UTF-8 and sets the bit. That case is
exactly the one the CP437 table exists for, so it has to be constructed
deliberately.

Run from the repository root:

    python3 packages/zip/oracle/fixtures.py

It rewrites `packages/zip/src/test_fixtures.mfb` and refreshes the archives in
`packages/zip/oracle/corpus/` (letter D's corpus reads that directory).
"""

import base64
import binascii
import io
import os
import struct
import subprocess
import sys
import tempfile
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
PKG = os.path.dirname(HERE)
CORPUS = os.path.join(HERE, "corpus")
OUT_MFB = os.path.join(PKG, "src", "test_fixtures.mfb")

# A fixed timestamp so regenerating produces identical bytes. Zip stores DOS
# time with 2-second granularity, so the seconds field is even.
FIXED_DATE = (2026, 9, 19, 12, 30, 0)


def zi(name, date=FIXED_DATE, method=zipfile.ZIP_DEFLATED):
    info = zipfile.ZipInfo(name, date_time=date)
    info.compress_type = method
    info.external_attr = 0o100644 << 16
    info.create_system = 3  # Unix
    return info


def build_simple():
    """Two files: one stored, one deflated (and genuinely compressible)."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr(zi("hello.txt", method=zipfile.ZIP_STORED), b"Hello, world!\n")
        z.writestr(zi("repeat.txt"), b"ab" * 512)
    return buf.getvalue()


def build_comment():
    """An archive comment, plus an entry, to exercise the EOCD comment scan.

    The comment is deliberately plain. An earlier draft embedded a `PK\\x05\\x06`
    lookalike to test that the scan picks the EOCD whose comment length lands
    exactly at end of file -- but Python's own reader refuses such an archive
    ("File is not a zip file"), so it cannot serve as an ORACLE fixture: there
    would be nothing to compare against. That adversarial case belongs in letter
    D's hand-damaged corpus, where "both refuse" is the expected outcome.
    """
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr(zi("only.txt", method=zipfile.ZIP_STORED), b"just one entry")
        z.comment = b"an archive comment"
    return buf.getvalue()


def build_dirs():
    """A directory entry beside a file inside it."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        d = zipfile.ZipInfo("sub/", date_time=FIXED_DATE)
        d.external_attr = (0o040755 << 16) | 0x10
        d.create_system = 3
        z.writestr(d, b"")
        z.writestr(zi("sub/inner.txt", method=zipfile.ZIP_STORED), b"inside a directory")
    return buf.getvalue()


def build_zip64():
    """Forced ZIP64 records on a small archive.

    force_zip64 makes the writer emit the ZIP64 extra field and the ZIP64 EOCD
    record/locator even though the sizes are small, which is the only way to
    test that arithmetic without writing 4 GiB.
    """
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", allowZip64=True) as z:
        info = zi("big.txt", method=zipfile.ZIP_STORED)
        with z.open(info, "w", force_zip64=True) as fh:
            fh.write(b"pretend this is enormous\n")
    return buf.getvalue()


def build_cp437():
    """A non-UTF-8 name with flag bit 11 CLEAR, built by hand.

    The name is `caf\x82.txt` -- byte 0x82 is U+00E9 (e-acute) in CP437. A
    reader that ignores the flag and assumes UTF-8 sees invalid UTF-8; a reader
    that applies the CP437 table sees `café.txt`.
    """
    name = b"caf\x82.txt"
    body = b"encoded in code page 437\n"
    crc = binascii.crc32(body) & 0xFFFFFFFF
    # DOS time/date for FIXED_DATE.
    dt = (FIXED_DATE[3] << 11) | (FIXED_DATE[4] << 5) | (FIXED_DATE[5] // 2)
    dd = ((FIXED_DATE[0] - 1980) << 9) | (FIXED_DATE[1] << 5) | FIXED_DATE[2]

    local = struct.pack(
        "<IHHHHHIIIHH",
        0x04034B50, 20, 0, 0, dt, dd, crc, len(body), len(body), len(name), 0
    ) + name
    offset = 0
    central = struct.pack(
        "<IHHHHHHIIIHHHHHII",
        0x02014B50, (3 << 8) | 20, 20, 0, 0, dt, dd, crc,
        len(body), len(body), len(name), 0, 0, 0, 0,
        0o100644 << 16, offset
    ) + name
    cd = central
    eocd = struct.pack(
        "<IHHHHIIH", 0x06054B50, 0, 0, 1, 1, len(cd),
        len(local) + len(body), 0
    )
    return local + body + cd + eocd


def build_ziptool():
    """An archive written by /usr/bin/zip, a third independent writer."""
    with tempfile.TemporaryDirectory() as tmp:
        src = os.path.join(tmp, "data")
        os.makedirs(src)
        with open(os.path.join(src, "one.txt"), "wb") as fh:
            fh.write(b"written by zip(1)\n")
        with open(os.path.join(src, "two.txt"), "wb") as fh:
            fh.write(b"xy" * 400)
        out = os.path.join(tmp, "made.zip")
        subprocess.run(
            ["/usr/bin/zip", "-q", "-X", "-r", out, "data"],
            cwd=tmp, check=True,
        )
        with open(out, "rb") as fh:
            return fh.read()


BUILDERS = [
    ("simple", build_simple),
    ("comment", build_comment),
    ("dirs", build_dirs),
    ("zip64", build_zip64),
    ("cp437", build_cp437),
    ("ziptool", build_ziptool),
]

# The one name zipfile reports differently from what the bytes say: it decodes
# a flag-11-clear name as CP437 only from Python 3.11 on, and our reader is the
# thing under test, so the expected name for `cp437` is pinned here explicitly.
NAME_OVERRIDE = {"cp437": {0: "café.txt"}}


def describe(label, data):
    """What the oracle says this archive contains."""
    entries = []
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        comment = z.comment
        for index, info in enumerate(z.infolist()):
            name = NAME_OVERRIDE.get(label, {}).get(index, info.filename)
            content = b"" if info.is_dir() else z.read(info)
            entries.append(
                {
                    "name": name,
                    "is_dir": info.is_dir(),
                    "method": info.compress_type,
                    "size": info.file_size,
                    "crc": info.CRC,
                    "content": content,
                }
            )
    return comment, entries


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


def emit():
    os.makedirs(CORPUS, exist_ok=True)
    chunks = []
    labels = []
    for label, build in BUILDERS:
        data = build()
        with open(os.path.join(CORPUS, label + ".zip"), "wb") as fh:
            fh.write(data)
        comment, entries = describe(label, data)
        labels.append(label)
        lines = []
        lines.append("PRIVATE FUNC fixture_%s() AS Fixture" % label)
        lines.append("  RETURN Fixture[%s, _" % mfb_string(label))
        lines.append("    encoding::base64Decode(%s), _" % mfb_string(base64.b64encode(data).decode()))
        lines.append("    %s, _" % mfb_string(comment.decode("utf-8", "replace")))
        if entries:
            lines.append("    [ _")
            rows = []
            for e in entries:
                rows.append(
                    "      ExpectedEntry[%s, %s, %d, %d, %d, %s]"
                    % (
                        mfb_string(e["name"]),
                        "TRUE" if e["is_dir"] else "FALSE",
                        e["method"],
                        e["size"],
                        e["crc"],
                        mfb_string(base64.b64encode(e["content"]).decode()),
                    )
                )
            # Every row continues, including the last: the closing bracket is on
            # its own line below.
            lines.append(", _\n".join(rows) + " _")
            lines.append("    ]]")
        else:
            lines.append("    []]")
        lines.append("END FUNC")
        chunks.append("\n".join(lines))

    header = '''\' ---------------------------------------------------------------------------
\' GENERATED by packages/zip/oracle/fixtures.py -- do not edit by hand.
\'
\' Each fixture is an archive written by an implementation that is not ours
\' (Python `zipfile`, or `/usr/bin/zip`, or -- for the CP437 case `zipfile`
\' cannot produce -- a hand-assembled header), together with the entry metadata
\' that implementation reports for it. The tests compare our reader against
\' these numbers, so a disagreement is a disagreement with the oracle rather
\' than with our own earlier output.
\'
\' Archive bytes are Base64 here rather than a byte-list literal: the six
\' fixtures total a few kilobytes, and a `[toByte(80), toByte(75), ...]` literal
\' of that size is thousands of calls to compile for no added clarity. The bytes
\' are the same either way.
\'
\' Regenerate with:  python3 packages/zip/oracle/fixtures.py
\' ---------------------------------------------------------------------------

IMPORT encoding

\' What the oracle says one entry of a fixture contains.
EXPORT TYPE ExpectedEntry
  name AS String
  isDirectory AS Boolean
  method AS Integer
  size AS Integer
  crc AS Integer
  contentBase64 AS String
END TYPE

\' One archive, plus what the oracle says is in it.
EXPORT TYPE Fixture
  label AS String
  bytes AS List OF Byte
  comment AS String
  entries AS List OF ExpectedEntry
END TYPE

'''
    body = "\n\n".join(chunks)
    listing = "EXPORT FUNC allFixtures() AS List OF Fixture\n  RETURN [ _\n" + ", _\n".join(
        "    fixture_%s()" % label for label in labels
    ) + " _\n  ]\nEND FUNC\n"
    with open(OUT_MFB, "w") as fh:
        fh.write(header + body + "\n\n" + listing)
    print("wrote %s (%d fixtures)" % (OUT_MFB, len(labels)))
    for label in labels:
        print("  corpus/%s.zip" % label)


if __name__ == "__main__":
    sys.exit(emit())
