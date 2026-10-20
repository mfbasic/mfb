#!/usr/bin/env python3
"""Compare packages/zip against Python's `zipfile`.

Four modes:

    corpus              the fixture archives, archives made by `zip -r`, and
                        hand-damaged variants of both
    fuzz --count N      the corpus with bytes, offsets and lengths mutated,
                        from a fixed seed so a failure can be reproduced
    roundtrip           archives this package WRITES, through both the Builder
                        and the streaming Writer, read back by Python zipfile
                        against ROUNDTRIP_ARCHIVES below
    extract             the corpus and the fuzz set EXTRACTED by the package
                        into fresh directories, judged against a Python model
                        of extraction that classifies names itself

Exit 0 means every archive agreed, or the difference is declared in
`divergences.json` with a reason. Anything else exits 1 and prints what
differed.

Two things are never declarable:

  * `sourcesAgree: false` -- the same archive read from a `List OF Byte` and
    from an `fs::File` giving different answers. That is not a difference of
    opinion with Python, it is the package contradicting itself, and it is the
    property the whole feature exists to provide.
  * a crash. A malformed archive must produce a refusal, not a panic.

Run from the repository root, after building the probes (see README.md):

    python3 packages/zip/oracle/diff.py corpus
"""

import argparse
import binascii
import io
import json
import os
import random
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import warnings
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
PKG = os.path.dirname(HERE)
ROOT = os.path.dirname(os.path.dirname(PKG))
CORPUS = os.path.join(HERE, "corpus")
PROBE = os.path.join(HERE, "probe", "build", "zipprobe.out")
WRITER = os.path.join(HERE, "writer-probe", "build", "zipwriter.out")
DIVERGENCES = os.path.join(HERE, "divergences.json")

# The two policies that may cover a refusal. Named once so the checks and the
# divergences file cannot drift apart silently.
POLICY_STRICTER = "policy:stricter-than-python"
POLICY_UNSAFE_NAMES = "policy:refuses-unsafe-names"
# The error names our extraction may refuse an unsafe archive with.
UNSAFE_REFUSALS = ("ErrInvalidPath", "ErrInvalidFormat")


def load_divergences():
    with open(DIVERGENCES) as fh:
        return json.load(fh)


def probe_lines(argv, count):
    """Run a probe, and return one parsed JSON object per output line."""
    done = subprocess.run(argv, capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit("probe exited %d\n%s" % (done.returncode, done.stderr))
    out = []
    for line in done.stdout.splitlines():
        line = line.strip()
        if line:
            out.append(json.loads(line))
    if len(out) != count:
        sys.exit("probe described %d of %d archives" % (len(out), count))
    return out


def run_probe(paths):
    """One parsed JSON object per archive, in order."""
    if not os.path.exists(PROBE):
        sys.exit("probe not built: mfb build %s" % os.path.join(HERE, "probe"))
    with tempfile.NamedTemporaryFile("w", suffix=".job", delete=False) as fh:
        fh.write("\n".join(paths) + "\n")
        job = fh.name
    try:
        return probe_lines([PROBE, job], len(paths))
    finally:
        os.unlink(job)


def run_extract_probe(paths, work):
    """The package extracts each archive into its own fresh directory under `work`."""
    if not os.path.exists(PROBE):
        sys.exit("probe not built: mfb build %s" % os.path.join(HERE, "probe"))
    with tempfile.NamedTemporaryFile("w", suffix=".job", delete=False) as fh:
        fh.write("\n".join(paths) + "\n")
        job = fh.name
    try:
        return probe_lines([PROBE, "extract", job, work], len(paths))
    finally:
        os.unlink(job)


def python_view(path):
    """What Python makes of the same archive: entries, or a refusal."""
    try:
        with zipfile.ZipFile(path) as z:
            entries = []
            for info in z.infolist():
                content_crc = None
                read_error = None
                # Directories are read too: Python checks a directory's local
                # header when it opens one, and that is compared like any entry.
                try:
                    content_crc = binascii.crc32(z.read(info)) & 0xFFFFFFFF
                except Exception as e:
                    read_error = "%s: %s" % (type(e).__name__, e)
                entries.append({
                    "name": info.filename,
                    # The name as written, before zipfile truncates at a NUL.
                    "raw": info.orig_filename,
                    "isDirectory": info.is_dir(),
                    "size": info.file_size,
                    "contentCrc": content_crc,
                    "readError": read_error,
                })
            return {"refused": False, "entries": entries}
    except Exception as e:
        return {"refused": True, "reason": type(e).__name__, "message": str(e)}


def compare(ours, theirs, declared):
    """A list of human-readable differences; empty when they agree."""
    path = ours["path"]
    problems = []

    if not ours.get("sourcesAgree", False):
        # Never declarable.
        problems.append("SOURCES DISAGREE: the List OF Byte and fs::File reads "
                        "gave different answers")
        return problems

    we_refused = "error" in ours
    they_refused = theirs["refused"]

    if we_refused and they_refused:
        return []            # both refuse: agreement

    if we_refused and not they_refused:
        # We are stricter. Declarable as a POLICY, not per-file: see
        # `policy:stricter-than-python` in divergences.json for why refusing a
        # self-inconsistent archive is the right answer for untrusted input.
        if POLICY_STRICTER in declared:
            return []
        problems.append("we refused (%s), Python accepted" % ours.get("error"))
        return problems

    if they_refused and not we_refused:
        # The dangerous direction, and never covered by the strictness policy:
        # we accepted something an independent reader would not.
        #
        # One narrow, named exception -- Python refusing on the ADVISORY
        # "version needed to extract" field. Keyed on Python's own message, so
        # it cannot quietly cover any other refusal.
        if theirs.get("message", "").startswith("zip file version") \
                and "policy:version-needed-is-advisory" in declared:
            return []
        key = "lenient:" + os.path.basename(path)
        if key in declared:
            return []
        problems.append("we accepted, Python refused (%s: %s)"
                        % (theirs.get("reason"), theirs.get("message", "")))
        return problems

    ours_entries = ours["archive"]["entries"]
    theirs_entries = theirs["entries"]
    if len(ours_entries) != len(theirs_entries):
        problems.append("entry count %d vs Python's %d"
                        % (len(ours_entries), len(theirs_entries)))
        return problems

    for index, (a, b) in enumerate(zip(ours_entries, theirs_entries)):
        where = "entry %d" % index
        if a["name"] != b["name"]:
            key = "name:%s:%s" % (a["name"], b["name"])
            if key not in declared:
                problems.append("%s name %r vs Python's %r"
                                % (where, a["name"], b["name"]))
                continue
        if a["size"] != b["size"]:
            problems.append("%s (%s) size %d vs Python's %d"
                            % (where, a["name"], a["size"], b["size"]))
        # A read failure on either side is only a problem when the other
        # succeeded: both failing is agreement about a broken entry.
        ours_failed = a["readError"] is not None
        theirs_failed = b["readError"] is not None
        if ours_failed and not theirs_failed:
            # Per-entry strictness, the same policy as the archive level: we
            # refused an entry Python was willing to hand back.
            if POLICY_STRICTER not in declared:
                problems.append("%s (%s) we refused the entry (%s), Python read it"
                                % (where, a["name"], a["readError"]))
        elif theirs_failed and not ours_failed:
            # The dangerous direction: we handed back an entry an independent
            # reader refused. Never covered by the policy.
            problems.append("%s (%s) we read the entry, Python refused it (%s)"
                            % (where, a["name"], b["readError"]))
        if not ours_failed and not theirs_failed and a["contentCrc"] != b["contentCrc"]:
            problems.append("%s (%s) contents differ (crc %s vs %s)"
                            % (where, a["name"], a["contentCrc"], b["contentCrc"]))
    return problems


def check(paths, declared, label):
    ours = run_probe(paths)
    failures = 0
    for entry in ours:
        problems = compare(entry, python_view(entry["path"]), declared)
        if problems:
            failures += 1
            print("FAIL %s" % entry["path"])
            for p in problems:
                print("     %s" % p)
    print("%s: %d archive(s), %d disagreement(s)" % (label, len(paths), failures))
    return failures


# --- extraction ----------------------------------------------------------------

def rel_of(raw):
    """The canonical relative path a name denotes: empty and "." components dropped.

    This is the package's `relPathOf` rule, applied to Python's raw name. A name
    that is unsafe keeps its ".." or leading "/" here, and is caught by
    `unsafe_classes` rather than silently normalised away.
    """
    return "/".join(part for part in raw.split("/") if part not in ("", "."))


def unsafe_classes(entries):
    """The name classes that make an archive one the policy refuses by design.

    These are the names an independent extractor sanitises or resolves rather
    than refuses: an absolute path, a ".." component, a backslash, a drive
    prefix, two entries that extract to one path, and a file where another
    entry needs a directory. Sorted, so the reason printed is stable.
    """
    found = set()
    files = set()
    dirs = set()
    for e in entries:
        raw = e["raw"]
        if "\\" in raw:
            found.add("backslash")
        if raw.startswith("/"):
            found.add("absolute path")
        if re.match(r"[A-Za-z]:", raw):
            found.add("drive letter")
        if ".." in raw.split("/"):
            found.add("'..' component")
        rel = rel_of(raw)
        if rel:
            parts = rel.split("/")
            for i in range(1, len(parts)):
                dirs.add("/".join(parts[:i]))
        if e["isDirectory"]:
            if rel:
                dirs.add(rel)
        else:
            if rel in files:
                found.add("duplicate name")
            files.add(rel)
    if files & dirs:
        found.add("file/directory collision")
    return sorted(found)


def python_refuses_extraction(entries):
    """Whether an independent extractor would stop on this archive.

    A file whose entry can't be read (a CRC mismatch, an unsupported method,
    encryption) stops it; so does a file named with no path component at all,
    which it would try to open as the directory it was given.
    """
    for e in entries:
        if e["isDirectory"]:
            continue
        if e["readError"] is not None or rel_of(e["raw"]) == "":
            return True
    return False


def read_refusal_codes(row):
    """The error codes the READ path refuses individual entries with.

    `row` is the reader probe's line for the same archive. Only per-entry refusals
    count: an archive the reader refuses outright is not a per-entry refusal.
    """
    codes = set()
    if "archive" in row:
        for x in row["archive"]["entries"]:
            if x["readError"] and x["readError"].startswith("read:"):
                codes.add(int(x["readError"][len("read:"):]))
    return codes


def compare_extract(ours, theirs, declared, read_row):
    """Problems with one archive's extraction, judged against Python's view.

    `read_row` is what the reader probe says about the same archive, so an
    extraction refusal can be checked against the read path's per-entry refusals.
    """
    path = ours["path"]
    problems = []

    if "refused" in ours:
        name = ours["refused"]
        if ours.get("leftover"):
            # Never declarable: the package promises a failed extraction leaves
            # nothing behind, and a leftover file is the gap that promise hides.
            problems.append("refused (%s) but left %d entr(ies) on disk: %s "
                            "(a failed extraction must leave nothing behind)"
                            % (name, len(ours["leftover"]), ours["leftover"]))
        if theirs["refused"]:
            return problems               # Python refuses the archive too
        if ours.get("stage") == "open":
            if POLICY_STRICTER in declared:
                return problems
            problems.append("we refused at open (%s), Python accepted" % name)
            return problems
        if python_refuses_extraction(theirs["entries"]):
            return problems               # Python refuses the extraction too
        classes = unsafe_classes(theirs["entries"])
        if classes:
            if name in UNSAFE_REFUSALS and POLICY_UNSAFE_NAMES in declared:
                return problems
            problems.append("we refused extraction (%s) on an archive with names the "
                            "policy covers (%s); that policy covers only %s, and only "
                            "once it is declared in divergences.json"
                            % (name, ", ".join(classes), "/".join(UNSAFE_REFUSALS)))
            return problems
        # The same per-entry refusal the READ path makes, of the same error code, on
        # this archive: the entry-level strictness that policy:stricter-than-python
        # already declares for reads. A refusal the read path does not make is not
        # covered, however strict extraction is about it.
        if POLICY_STRICTER in declared and ours["code"] in read_refusal_codes(read_row):
            return problems
        problems.append("we refused extraction (%s, code %d); Python extracts this archive "
                        "cleanly, and the read path does not refuse it with that code (%s)"
                        % (name, ours["code"], sorted(read_refusal_codes(read_row)) or "no per-entry refusal"))
        return problems

    # We wrote files.
    if theirs["refused"]:
        if theirs.get("message", "").startswith("zip file version") \
                and "policy:version-needed-is-advisory" in declared:
            return problems
        if ("lenient:" + os.path.basename(path)) in declared:
            return problems
        problems.append("we extracted, Python refused the archive (%s: %s)"
                        % (theirs.get("reason"), theirs.get("message", "")))
        return problems

    classes = unsafe_classes(theirs["entries"])
    if classes:
        problems.append("we extracted an archive with names the policy refuses: %s"
                        % ", ".join(classes))
        return problems
    if python_refuses_extraction(theirs["entries"]):
        problems.append("we extracted entries Python cannot read or place")
        return problems

    names = [f["path"] for f in ours["files"]]
    if names != sorted(names):
        problems.append("files are not sorted: %r" % names)
    if ours["written"] != len(ours["files"]):
        problems.append("written %d but %d files on disk"
                        % (ours["written"], len(ours["files"])))

    ours_map = {f["path"]: (f["size"], f["crc"]) for f in ours["files"]}
    theirs_map = {}
    for e in theirs["entries"]:
        if not e["isDirectory"]:
            theirs_map[rel_of(e["raw"])] = (e["size"], e["contentCrc"])
    for p in sorted(set(ours_map) - set(theirs_map)):
        problems.append("on disk after our extraction, not in Python's view: %r" % p)
    for p in sorted(set(theirs_map) - set(ours_map)):
        problems.append("in Python's view, not on disk after our extraction: %r" % p)
    for p in sorted(set(ours_map) & set(theirs_map)):
        if ours_map[p] != theirs_map[p]:
            problems.append("%r: our (size, crc) %s, Python's %s"
                            % (p, ours_map[p], theirs_map[p]))
    return problems


def check_extract(paths, declared, work):
    extract_root = os.path.join(work, "extracted")
    os.makedirs(extract_root)
    ours = run_extract_probe(paths, extract_root)
    read_rows = run_probe(paths)
    failures = 0
    for entry, read_row in zip(ours, read_rows):
        problems = compare_extract(entry, python_view(entry["path"]), declared, read_row)
        if problems:
            failures += 1
            print("FAIL %s" % entry["path"])
            for p in problems:
                print("     %s" % p)
    print("extract: %d archive(s), %d disagreement(s)" % (len(paths), failures))
    return failures


# --- roundtrip -----------------------------------------------------------------

# What the writer probe must produce, and what Python must find in each archive.
#
# These values are Python's, not the package's: the probe writes them, and this
# table is what it is checked against. packages/zip/oracle/writer-probe/src/main.mfb
# carries the same names, contents, times, modes and comment. Change both
# together; a drift shows up as a failed check, never as a silent pass.

T_EVEN = 1789000000      # even seconds
T_ODD = 1789000001       # odd seconds: DOS time keeps 40 s, the extended field 41 s

MIXED_COMMENT = "built by the zip writer probe, é".encode("utf-8")


def expect_entry(name, content=b"", mtime=T_EVEN, mode=None, method=None, directory=False):
    """One expected entry. `method` is "stored", "deflated" or None (not checked)."""
    return {"name": name, "content": content, "mtime": mtime, "mode": mode,
            "method": method, "dir": directory}


def mixed_entries():
    return [
        expect_entry("stored.txt", b"stored text\n", method="stored"),
        expect_entry("deflated.txt", b"deflate me " * 100, method="deflated"),
        expect_entry("empty.txt", b"", method="stored"),
        # 256 distinct bytes: deflate cannot make them smaller, so the rule
        # "deflated only when that makes it smaller" stores them.
        expect_entry("incompressible.bin", bytes(range(256)), method="stored"),
        expect_entry("dir/", mode=0o750, directory=True),
        expect_entry("dir/ünïcode-名前.txt", "naïve\n".encode("utf-8")),
        expect_entry("odd-time.txt", b"odd", mtime=T_ODD, mode=0o600),
        expect_entry("even-time.txt", b"even"),
        expect_entry("tool.sh", b"#!/bin/sh\necho hi\n", mode=0o755),
    ]


def entry_count_entries():
    # 65536 entries: one more than the classic 16-bit count holds, so ZIP64 is needed.
    return [expect_entry("n%d.txt" % i, ("entry %d" % i).encode("ascii"), mtime=0)
            for i in range(65536)]


def roundtrip_expectations():
    mixed = mixed_entries()
    return {
        "builder-mixed.zip": (mixed, MIXED_COMMENT),
        "stream-mixed.zip": (mixed, MIXED_COMMENT),
        "builder-empty.zip": ([], b""),
        "stream-empty.zip": ([], b""),
        "entry-count-65536.zip": (entry_count_entries(), b""),
    }


# Pairs that must be byte-identical: the Builder and the Writer, given the same calls.
SAME_BYTES = [("builder-mixed.zip", "stream-mixed.zip"),
              ("builder-empty.zip", "stream-empty.zip")]


def dos_expected(mtime):
    """The DOS date and time Python should report for a Unix time.

    DOS holds two seconds, rounded down; an unrecorded time (0) is the earliest
    instant the format names, 1980-01-01 00:00:00.
    """
    if mtime == 0:
        return (1980, 1, 1, 0, 0, 0)
    t = time.gmtime(mtime)
    return (t.tm_year, t.tm_mon, t.tm_mday, t.tm_hour, t.tm_min, t.tm_sec - t.tm_sec % 2)


def extended_times(extra):
    """The exact Unix times in an entry's extended-timestamp (0x5455) fields."""
    found = []
    i = 0
    while i + 4 <= len(extra):
        tag, size = struct.unpack_from("<HH", extra, i)
        body = extra[i + 4:i + 4 + size]
        if tag == 0x5455:
            if len(body) >= 5 and body[0] & 1:
                found.append(struct.unpack_from("<I", body, 1)[0])
            else:
                found.append(None)
        i += 4 + size
    return found


def check_entry(z, info, want):
    problems = []
    name = want["name"]
    if any(ord(c) > 127 for c in name) and not info.flag_bits & 0x800:
        problems.append("non-ASCII name without the UTF-8 flag (bit 11)")
    if info.is_dir() != want["dir"]:
        problems.append("is_dir() is %s, expected %s" % (info.is_dir(), want["dir"]))
    if info.file_size != len(want["content"]):
        problems.append("size %d, expected %d" % (info.file_size, len(want["content"])))
    if not want["dir"]:
        try:
            data = z.read(info)
        except Exception as e:
            problems.append("read failed: %s: %s" % (type(e).__name__, e))
            data = None
        if data is not None and data != want["content"]:
            problems.append("content differs from what was written")
        if info.CRC != binascii.crc32(want["content"]) & 0xFFFFFFFF:
            problems.append("CRC %08x, expected %08x"
                            % (info.CRC, binascii.crc32(want["content"]) & 0xFFFFFFFF))
        if info.compress_type == zipfile.ZIP_STORED:
            if info.compress_size != info.file_size:
                problems.append("stored, but compressed size %d != size %d"
                                % (info.compress_size, info.file_size))
        elif info.compress_type == zipfile.ZIP_DEFLATED:
            if not info.compress_size < info.file_size:
                problems.append("deflated, but not smaller (%d >= %d)"
                                % (info.compress_size, info.file_size))
        else:
            problems.append("compression method %d" % info.compress_type)
        if want["method"] is not None:
            expect = zipfile.ZIP_STORED if want["method"] == "stored" else zipfile.ZIP_DEFLATED
            if info.compress_type != expect:
                problems.append("method %d, expected %s" % (info.compress_type, want["method"]))
    if info.date_time != dos_expected(want["mtime"]):
        problems.append("DOS date_time %r, expected %r"
                        % (info.date_time, dos_expected(want["mtime"])))
    times = extended_times(info.extra)
    if want["mtime"] == 0:
        if times:
            problems.append("extended timestamp present for an unrecorded time: %r" % times)
    elif times != [want["mtime"]]:
        problems.append("extended timestamp %r, expected [%d]" % (times, want["mtime"]))
    if info.create_system != 3:
        problems.append("create_system %d, expected 3 (Unix)" % info.create_system)
    attr = info.external_attr
    kind = (attr >> 16) & 0o170000
    perm = (attr >> 16) & 0o7777
    want_kind = 0o040000 if want["dir"] else 0o100000
    if want["mode"] is not None:
        want_perm = want["mode"]
    else:
        want_perm = 0o755 if want["dir"] else 0o644
    if kind != want_kind:
        problems.append("file type bits %o, expected %o" % (kind, want_kind))
    if perm != want_perm:
        problems.append("permission bits %o, expected %o" % (perm, want_perm))
    if want["dir"] and not attr & 0x10:
        problems.append("directory without the DOS directory attribute")
    return problems


def check_written(path, entries, comment):
    try:
        z = zipfile.ZipFile(path)
    except Exception as e:
        return ["Python refused the archive: %s: %s" % (type(e).__name__, e)]
    problems = []
    with z:
        bad = z.testzip()
        if bad is not None:
            problems.append("testzip: first bad entry %r" % bad)
        infos = z.infolist()
        want_names = [w["name"] for w in entries]
        got_names = [i.filename for i in infos]
        if got_names != want_names:
            if len(got_names) != len(want_names):
                problems.append("%d entries, expected %d" % (len(got_names), len(want_names)))
            for i, (g, w) in enumerate(zip(got_names, want_names)):
                if g != w:
                    problems.append("entry %d named %r, expected %r" % (i, g, w))
                    break
        if z.comment != comment:
            problems.append("comment %r, expected %r" % (z.comment, comment))
        for info, want in zip(infos, entries):
            for p in check_entry(z, info, want):
                problems.append("%s: %s" % (info.filename, p))
    return problems


def build_roundtrip(into):
    """Run the writer probe into `into`. The probe is the only source of archives."""
    if not os.path.exists(WRITER):
        sys.exit("writer probe not built: mfb build %s"
                 % os.path.join(HERE, "writer-probe"))
    done = subprocess.run([WRITER, into], capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit("writer probe exited %d\n%s" % (done.returncode, done.stderr))


def check_roundtrip(into):
    build_roundtrip(into)
    expected = roundtrip_expectations()
    present = sorted(n for n in os.listdir(into) if n.endswith(".zip"))
    failing = set()
    if present != sorted(expected):
        print("FAIL writer probe produced %s, expected %s" % (present, sorted(expected)))
        failing.update(set(present) ^ set(expected))
    for name in sorted(expected):
        path = os.path.join(into, name)
        if not os.path.exists(path):
            continue
        entries, comment = expected[name]
        problems = check_written(path, entries, comment)
        if problems:
            failing.add(name)
            print("FAIL %s" % path)
            for p in problems:
                print("     %s" % p)
    for first, second in SAME_BYTES:
        a = os.path.join(into, first)
        b = os.path.join(into, second)
        if not (os.path.exists(a) and os.path.exists(b)):
            continue
        with open(a, "rb") as fa, open(b, "rb") as fb:
            if fa.read() != fb.read():
                failing.add(second)
                print("FAIL %s" % b)
                print("     bytes differ from %s, but the Builder and the Writer "
                      "given the same calls must write the same archive" % first)
    print("roundtrip: %d archive(s), %d disagreement(s)"
          % (len(expected), len(failing)))
    return len(failing)


# --- corpus construction --------------------------------------------------

def damaged_variants(src, into):
    """Hand-damaged copies: the ways a zip goes wrong that a reader must survive."""
    with open(src, "rb") as fh:
        data = fh.read()
    base = os.path.splitext(os.path.basename(src))[0]
    made = []

    def write(suffix, blob):
        path = os.path.join(into, "%s-%s.zip" % (base, suffix))
        with open(path, "wb") as fh:
            fh.write(blob)
        made.append(path)

    # Truncated at five points across the archive.
    for i in range(1, 6):
        cut = len(data) * i // 6
        write("trunc%d" % i, data[:cut])
    if len(data) >= 22:
        eocd = len(data) - 22
        # EOCD comment length off by one: the record no longer ends at EOF.
        bad = bytearray(data)
        bad[eocd + 20] = 1
        write("commentlen", bytes(bad))
        # Central directory offset past the end.
        bad = bytearray(data)
        bad[eocd + 16:eocd + 20] = (0xFFFFFF00).to_bytes(4, "little")
        write("cdpast", bytes(bad))
        # Entry count larger than the directory holds.
        bad = bytearray(data)
        bad[eocd + 10:eocd + 12] = (999).to_bytes(2, "little")
        write("count", bytes(bad))
        # A ZIP64 locator signature immediately before the EOCD, pointing past EOF.
        if eocd >= 20:
            bad = bytearray(data)
            bad[eocd - 20:eocd - 16] = b"PK\x06\x07"
            bad[eocd - 12:eocd - 4] = (0xFFFFFFFF).to_bytes(8, "little")
            write("zip64past", bytes(bad))
    return made


def build_corpus(into):
    paths = []
    for name in sorted(os.listdir(CORPUS)):
        if name.endswith(".zip"):
            src = os.path.join(CORPUS, name)
            dst = os.path.join(into, name)
            shutil.copyfile(src, dst)
            paths.append(dst)
    # An archive made by zip(1) out of real source files.
    made = os.path.join(into, "ziptool-tree.zip")
    subprocess.run(["/usr/bin/zip", "-q", "-r", made, "packages/jwt/src"],
                   cwd=ROOT, check=True)
    shutil.move(os.path.join(ROOT, made) if not os.path.isabs(made) else made,
                os.path.join(into, "ziptool-tree.zip"))
    paths.append(os.path.join(into, "ziptool-tree.zip"))

    for src in list(paths):
        paths.extend(damaged_variants(src, into))
    return paths


def build_fuzz(into, count, seed):
    """Mutate corpus archives: flipped bytes, and clobbered length fields."""
    rng = random.Random(seed)
    seeds = [os.path.join(CORPUS, n) for n in sorted(os.listdir(CORPUS))
             if n.endswith(".zip")]
    blobs = []
    for path in seeds:
        with open(path, "rb") as fh:
            blobs.append(fh.read())
    paths = []
    for i in range(count):
        data = bytearray(rng.choice(blobs))
        if not data:
            continue
        for _ in range(rng.randint(1, 4)):
            kind = rng.random()
            at = rng.randrange(len(data))
            if kind < 0.6:
                data[at] = rng.randrange(256)
            elif kind < 0.85 and len(data) >= 4:
                at = min(at, len(data) - 4)
                data[at:at + 4] = rng.randrange(1 << 32).to_bytes(4, "little")
            else:
                at = min(at, len(data) - 2)
                data[at:at + 2] = rng.randrange(1 << 16).to_bytes(2, "little")
        path = os.path.join(into, "fuzz%05d.zip" % i)
        with open(path, "wb") as fh:
            fh.write(bytes(data))
        paths.append(path)
    return paths


def build_hostile(into):
    """Archives whose NAMES are the question, written by Python zipfile.

    Each unsafe archive must be refused by the package, and each is one of the
    name classes the policy covers. The last archive is safe but unusual: its
    "./" and empty components are dropped by the package, so it must extract to
    exactly the paths Python's view names after the same normalisation.
    """
    cases = {
        "hostile-dotdot.zip": ["../escape.txt"],
        "hostile-absolute.zip": ["/absolute.txt"],
        "hostile-backslash.zip": ["dir\\file.txt"],
        "hostile-drive.zip": ["C:drive.txt"],
        "hostile-duplicate.zip": ["same.txt", "same.txt"],
        "hostile-collision.zip": ["clash", "clash/inner.txt"],
        "hostile-dirfile.zip": ["d/", "d"],
        "normalised-names.zip": ["./a//b.txt", "a/./c.txt", "dir/"],
    }
    paths = []
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")      # zipfile warns about the duplicate on purpose
        for name, names in sorted(cases.items()):
            path = os.path.join(into, name)
            with zipfile.ZipFile(path, "w") as z:
                for member in names:
                    z.writestr(member, b"payload for " + member.encode("utf-8"))
            paths.append(path)
    return paths


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", choices=["corpus", "fuzz", "roundtrip", "extract"])
    ap.add_argument("--count", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=139)
    args = ap.parse_args()

    declared = load_divergences()
    with tempfile.TemporaryDirectory() as work:
        if args.mode == "corpus":
            return 1 if check(build_corpus(work), declared, args.mode) else 0
        if args.mode == "fuzz":
            return 1 if check(build_fuzz(work, args.count, args.seed), declared, args.mode) else 0
        if args.mode == "roundtrip":
            return 1 if check_roundtrip(work) else 0
        paths = build_corpus(work) + build_fuzz(work, args.count, args.seed) + build_hostile(work)
        return 1 if check_extract(paths, declared, work) else 0


if __name__ == "__main__":
    sys.exit(main())
