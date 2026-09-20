#!/usr/bin/env python3
"""Compare packages/zip against Python's `zipfile` over a corpus.

Three modes:

    corpus              the fixture archives, archives made by `zip -r`, and
                        hand-damaged variants of both
    fuzz --count N      the corpus with bytes, offsets and lengths mutated,
                        from a fixed seed so a failure can be reproduced
    roundtrip           archives this package WRITES, read back by Python

Exit 0 means every archive agreed, or the difference is declared in
`divergences.json` with a reason. Anything else exits 1 and prints what
differed.

Two things are never declarable:

  * `sourcesAgree: false` -- the same archive read from a `List OF Byte` and
    from an `fs::File` giving different answers. That is not a difference of
    opinion with Python, it is the package contradicting itself, and it is the
    property the whole feature exists to provide.
  * a crash. A malformed archive must produce a refusal, not a panic.

Run from the repository root:

    python3 packages/zip/oracle/diff.py corpus
"""

import argparse
import binascii
import io
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
PKG = os.path.dirname(HERE)
ROOT = os.path.dirname(os.path.dirname(PKG))
CORPUS = os.path.join(HERE, "corpus")
PROBE = os.path.join(HERE, "probe", "build", "zipprobe.out")
DIVERGENCES = os.path.join(HERE, "divergences.json")


def load_divergences():
    with open(DIVERGENCES) as fh:
        return json.load(fh)


def run_probe(paths):
    """One parsed JSON object per archive, in order."""
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
    out = []
    for line in done.stdout.splitlines():
        line = line.strip()
        if line:
            out.append(json.loads(line))
    return out


def python_view(path):
    """What Python makes of the same archive: entries, or a refusal."""
    try:
        with zipfile.ZipFile(path) as z:
            entries = []
            for info in z.infolist():
                content_crc = None
                read_error = None
                if info.is_dir():
                    content_crc = 0
                else:
                    try:
                        content_crc = binascii.crc32(z.read(info)) & 0xFFFFFFFF
                    except Exception as e:
                        read_error = "%s: %s" % (type(e).__name__, e)
                entries.append({
                    "name": info.filename,
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
        if "policy:stricter-than-python" in declared:
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


def build_roundtrip(into):
    """Archives this package writes, for Python to judge."""
    writer = os.path.join(HERE, "probe", "build", "zipwriter.out")
    if not os.path.exists(writer):
        # The writer probe is optional; roundtrip falls back to /tmp/zipw if a
        # previous probe run left archives there.
        legacy = "/tmp/zipw"
        if os.path.isdir(legacy):
            out = []
            for name in sorted(os.listdir(legacy)):
                if name.endswith(".zip"):
                    dst = os.path.join(into, name)
                    shutil.copyfile(os.path.join(legacy, name), dst)
                    out.append(dst)
            return out
        sys.exit("roundtrip needs archives written by the package; run the "
                 "writer probe first (see README)")
    subprocess.run([writer, into], check=True)
    return [os.path.join(into, n) for n in sorted(os.listdir(into))
            if n.endswith(".zip")]


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
