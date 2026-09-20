#!/usr/bin/env python3
"""Compare packages/tar against Python's `tarfile` over a corpus.

Three modes:

    corpus              the fixture archives, an archive made by `bsdtar -cf`,
                        and hand-damaged variants of both
    fuzz --count N      the corpus with bytes and header fields mutated, from a
                        fixed seed so a failure can be reproduced
    roundtrip           archives this package WRITES, read back by Python

Exit 0 means every archive agreed, or the difference is declared in
`divergences.json` with a reason.

Two things are never declarable:

  * `sourcesAgree: false` -- the same archive read from a `List OF Byte` and
    from an `fs::File` giving different answers. That is the package
    contradicting itself, which is the one property this whole feature exists
    to provide.
  * a crash. A malformed archive must produce a refusal, not a panic.

Run from the repository root:

    python3 packages/tar/oracle/diff.py corpus
"""

import argparse
import binascii
import json
import os
import random
import shutil
import subprocess
import sys
import tarfile
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PKG = os.path.dirname(HERE)
ROOT = os.path.dirname(os.path.dirname(PKG))
CORPUS = os.path.join(HERE, "corpus")
PROBE = os.path.join(HERE, "probe", "build", "tarprobe.out")
DIVERGENCES = os.path.join(HERE, "divergences.json")

KIND = {
    tarfile.REGTYPE: 0, tarfile.AREGTYPE: 0,
    tarfile.DIRTYPE: 1, tarfile.SYMTYPE: 2, tarfile.LNKTYPE: 3,
}


def load_divergences():
    with open(DIVERGENCES) as fh:
        return json.load(fh)


def run_probe(paths):
    if not os.path.exists(PROBE):
        sys.exit("probe not built: mfb build %s" % os.path.join(HERE, "probe"))
    with tempfile.NamedTemporaryFile("w", suffix=".job", delete=False) as fh:
        fh.write("\n".join(paths) + "\n")
        job = fh.name
    try:
        done = subprocess.run([PROBE, job], capture_output=True, text=True)
    finally:
        os.unlink(job)
    if done.returncode != 0:
        sys.exit("probe exited %d\n%s" % (done.returncode, done.stderr))
    return [json.loads(l) for l in done.stdout.splitlines() if l.strip()]


def python_view(path):
    try:
        with tarfile.open(path) as t:
            entries = []
            for info in t.getmembers():
                content_crc = 0
                read_error = None
                if info.isreg():
                    try:
                        fh = t.extractfile(info)
                        content_crc = binascii.crc32(fh.read() if fh else b"") & 0xFFFFFFFF
                    except Exception as e:
                        read_error = type(e).__name__
                        content_crc = None
                name = info.name
                if info.isdir() and not name.endswith("/"):
                    name += "/"
                entries.append({
                    "name": name,
                    "kind": KIND.get(info.type, 4),
                    "size": info.size,
                    "contentCrc": content_crc,
                    "readError": read_error,
                })
            return {"refused": False, "entries": entries}
    except Exception as e:
        return {"refused": True, "reason": type(e).__name__}


def compare(ours, theirs, declared):
    path = ours["path"]
    problems = []

    if not ours.get("sourcesAgree", False):
        problems.append("SOURCES DISAGREE: the List OF Byte and fs::File reads "
                        "gave different answers")
        return problems

    we_refused = "error" in ours
    they_refused = theirs["refused"]

    if we_refused and they_refused:
        return []

    if we_refused and not they_refused:
        # We are stricter. A policy, not a per-file excuse -- see
        # `policy:stricter-than-python` in divergences.json.
        if "policy:stricter-than-python" in declared:
            return []
        problems.append("we refused (%s), Python accepted" % ours.get("error"))
        return problems

    if they_refused and not we_refused:
        # The dangerous direction: we accepted what an independent reader would
        # not. Never covered by the policy above.
        key = "lenient:" + os.path.basename(path)
        if key in declared:
            return []
        problems.append("we accepted, Python refused (%s)" % theirs.get("reason"))
        return problems

    ours_entries = ours["archive"]["entries"]
    theirs_entries = theirs["entries"]
    if len(ours_entries) != len(theirs_entries):
        # Finding MORE entries than Python is the declarable direction: Python's
        # tarfile stops listing at the first header it dislikes and reports what
        # it has, rather than raising. Finding FEWER is never declarable -- that
        # would mean we are the ones missing entries.
        if len(ours_entries) > len(theirs_entries) \
                and "policy:python-stops-early" in declared:
            return []
        key = "count:" + os.path.basename(path)
        if key not in declared:
            problems.append("entry count %d vs Python's %d"
                            % (len(ours_entries), len(theirs_entries)))
        return problems

    for index, (a, b) in enumerate(zip(ours_entries, theirs_entries)):
        where = "entry %d" % index
        if a["name"] != b["name"]:
            # Tar records no character set for a name, so a name that is not
            # valid UTF-8 has no single right presentation. Python marks those
            # bytes with surrogate escapes (U+DC80..U+DCFF); we map them through
            # Latin-1. The BYTES agree -- only the rendering differs -- so this
            # is declared when Python's name carries a surrogate, and only then.
            python_escaped = any(0xDC80 <= ord(ch) <= 0xDCFF for ch in b["name"])
            if python_escaped and "policy:name-encoding-unspecified" in declared:
                continue
            key = "name:%s:%s" % (a["name"], b["name"])
            if key not in declared:
                problems.append("%s name %r vs Python's %r"
                                % (where, a["name"], b["name"]))
                continue
        if a["kind"] != b["kind"]:
            problems.append("%s (%s) kind %d vs Python's %d"
                            % (where, a["name"], a["kind"], b["kind"]))
        if a["size"] != b["size"]:
            problems.append("%s (%s) size %d vs Python's %d"
                            % (where, a["name"], a["size"], b["size"]))
        ours_failed = a["readError"] is not None
        theirs_failed = b["readError"] is not None
        if ours_failed and not theirs_failed:
            # Per-entry strictness, the same policy as the archive level: we
            # refused an entry Python was willing to hand back.
            if "policy:stricter-than-python" not in declared:
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
    if len(ours) != len(paths):
        sys.exit("probe described %d of %d archives" % (len(ours), len(paths)))
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


# --- corpus construction --------------------------------------------------

def damaged_variants(src, into):
    """The ways a tar goes wrong that a reader must survive."""
    with open(src, "rb") as fh:
        data = fh.read()
    base = os.path.splitext(os.path.basename(src))[0]
    made = []

    def write(suffix, blob):
        path = os.path.join(into, "%s-%s.tar" % (base, suffix))
        with open(path, "wb") as fh:
            fh.write(blob)
        made.append(path)

    for i in range(1, 6):
        write("trunc%d" % i, data[:len(data) * i // 6])
    if len(data) >= 512:
        # A flipped checksum digit.
        bad = bytearray(data)
        bad[148] = ord("7") if bad[148] != ord("7") else ord("1")
        write("checksum", bytes(bad))
        # A size field that is not octal.
        bad = bytearray(data)
        bad[124:136] = b"99999999999\x00"
        write("badsize", bytes(bad))
        # A size claiming far more data than the archive holds.
        bad = bytearray(data)
        bad[124:136] = b"77777777777\x00"
        write("hugesize", bytes(bad))
        # A name field with no NUL terminator at all.
        bad = bytearray(data)
        bad[0:100] = b"A" * 100
        write("unterminated", bytes(bad))
    return made


def build_corpus(into):
    paths = []
    for name in sorted(os.listdir(CORPUS)):
        if name.endswith(".tar"):
            dst = os.path.join(into, name)
            shutil.copyfile(os.path.join(CORPUS, name), dst)
            paths.append(dst)
    made = os.path.join(into, "bsdtar-tree.tar")
    subprocess.run(["/usr/bin/bsdtar", "-cf", made, "packages/jwt/src"],
                   cwd=ROOT, check=True)
    paths.append(made)
    for src in list(paths):
        paths.extend(damaged_variants(src, into))
    return paths


def build_fuzz(into, count, seed):
    rng = random.Random(seed)
    blobs = []
    for name in sorted(os.listdir(CORPUS)):
        if name.endswith(".tar"):
            with open(os.path.join(CORPUS, name), "rb") as fh:
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
            elif kind < 0.85:
                # Clobber a header numeric field with octal-looking digits.
                block = (at // 512) * 512
                field = block + rng.choice([100, 108, 116, 124, 136, 148])
                if field + 12 <= len(data):
                    digits = "".join(rng.choice("01234567") for _ in range(11))
                    data[field:field + 12] = digits.encode() + b"\x00"
            else:
                # Clobber a whole name field.
                block = (at // 512) * 512
                if block + 100 <= len(data):
                    data[block:block + 100] = bytes(rng.randrange(256) for _ in range(100))
        path = os.path.join(into, "fuzz%05d.tar" % i)
        with open(path, "wb") as fh:
            fh.write(bytes(data))
        paths.append(path)
    return paths


def build_roundtrip(into):
    legacy = "/tmp/tarw"
    if os.path.isdir(legacy):
        out = []
        for name in sorted(os.listdir(legacy)):
            if name.endswith(".tar"):
                dst = os.path.join(into, name)
                shutil.copyfile(os.path.join(legacy, name), dst)
                out.append(dst)
        if out:
            return out
    sys.exit("roundtrip needs archives written by the package; run the writer "
             "probe first (see README)")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("mode", choices=["corpus", "fuzz", "roundtrip"])
    ap.add_argument("--count", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=139)
    args = ap.parse_args()

    declared = load_divergences()
    with tempfile.TemporaryDirectory() as work:
        if args.mode == "corpus":
            paths = build_corpus(work)
        elif args.mode == "fuzz":
            paths = build_fuzz(work, args.count, args.seed)
        else:
            paths = build_roundtrip(work)
        failures = check(paths, declared, args.mode)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
