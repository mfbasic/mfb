#!/usr/bin/env python3
"""Generate packages/timezones/src/data.mfb from the vendored IANA tzdb release.

    python3 tools/tzdb/gen_timezones_data.py > packages/timezones/src/data.mfb

The artifact goes to stdout and a statistics line goes to stderr.
`scripts/check-generated.sh` re-runs this script and fails on any difference.

Pipeline (plan-135-A § 4.2):
  1. check both tarballs against third_party/tzdb/<release>/SHA256SUMS;
  2. unpack both into one temporary directory, then run `make zic`;
  3. `zic -b slim` over the release's default source set;
  4. parse every compiled TZif file per RFC 8536;
  5. group names whose TZif bytes are identical, so a link shares its target's data;
  6. fail closed on any input that packages/timezones does not handle;
  7. emit data.mfb.

Every premise that plan-135-B's footer evaluator and plan-135-C's `civil` window
rely on is asserted in step 6. A future release that breaks one fails here, not
at runtime.
"""

import hashlib
import os
import re
import struct
import subprocess
import sys
import tarfile
import tempfile

RELEASE = "2026d"
SOURCES = [
    "africa", "antarctica", "asia", "australasia", "europe",
    "northamerica", "southamerica", "etcetera", "backward", "factory",
]

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
VENDOR = os.path.join(ROOT, "third_party", "tzdb", RELEASE)
TARBALLS = ["tzdata%s.tar.gz" % RELEASE, "tzcode%s.tar.gz" % RELEASE]

# plan-135-C's `civil` probes offsets at L - W and L + W with W = 172800. The
# window holds at most one transition only if transitions are more than 2W
# apart, and it holds both candidates only if every |utoff| < W.
MAX_ABS_UTOFF = 172800
MIN_TRANSITION_GAP = 345600

DESIGNATION = re.compile(r"^[A-Za-z0-9+-]+$")
# std/dst = "<" 1*(ALPHA / DIGIT / "+" / "-") ">" / 3*ALPHA
FOOTER_NAME = r"(?:<[A-Za-z0-9+-]+>|[A-Za-z]{3,})"
FOOTER_OFFSET = r"[+-]?\d{1,2}(?::\d{2}(?::\d{2})?)?"
FOOTER_RULE = r"M\d{1,2}\.\d\.\d(?:/[+-]?\d{1,3}(?::\d{2}(?::\d{2})?)?)?"
FOOTER = re.compile(
    r"^(?P<std>%s)(?P<stdoff>%s)(?:(?P<dst>%s)(?P<dstoff>%s)?(?:,(?P<start>[^,]+),(?P<end>[^,]+))?)?$"
    % (FOOTER_NAME, FOOTER_OFFSET, FOOTER_NAME, FOOTER_OFFSET)
)


def fail(message):
    sys.stderr.write("gen_timezones_data: %s\n" % message)
    sys.exit(1)


def verify_checksums():
    expected = {}
    with open(os.path.join(VENDOR, "SHA256SUMS"), encoding="ascii") as f:
        for line in f:
            digest, name = line.split()
            expected[name] = digest
    for name in TARBALLS:
        with open(os.path.join(VENDOR, name), "rb") as f:
            actual = hashlib.sha256(f.read()).hexdigest()
        if expected.get(name) != actual:
            fail("checksum mismatch for %s: SHA256SUMS has %s, file hashes to %s"
                 % (name, expected.get(name), actual))


def extract(tarball, dest):
    with tarfile.open(tarball) as archive:
        if hasattr(tarfile, "data_filter"):
            archive.extractall(dest, filter="data")
        else:
            archive.extractall(dest)


def compile_zones(work):
    for name in TARBALLS:
        extract(os.path.join(VENDOR, name), work)
    with open(os.path.join(work, "version"), encoding="ascii") as f:
        version = f.read().strip()
    if version != RELEASE:
        fail("the tarballs are release %s, but RELEASE is %s" % (version, RELEASE))
    env = dict(os.environ, LC_ALL="C")
    subprocess.run(["make", "zic"], cwd=work, env=env, check=True,
                   stdout=sys.stderr, stderr=sys.stderr)
    out = os.path.join(work, "out")
    subprocess.run(["./zic", "-b", "slim", "-d", out] + SOURCES, cwd=work, env=env,
                   check=True, stdout=sys.stderr, stderr=sys.stderr)
    return out


def parse_tzif(name, data):
    """Return (types, transitions, footer) from RFC 8536 version 2+ TZif bytes."""
    header = struct.Struct(">4sc15x6L")

    def read_header(offset):
        if len(data) < offset + header.size:
            fail("%s: truncated TZif header" % name)
        magic, version, isut, isstd, leap, timecnt, typecnt, charcnt = header.unpack_from(data, offset)
        if magic != b"TZif":
            fail("%s: not a TZif file" % name)
        return version, isut, isstd, leap, timecnt, typecnt, charcnt

    version, isut, isstd, leap, timecnt, typecnt, charcnt = read_header(0)
    if version not in (b"2", b"3", b"4"):
        fail("%s: TZif version %r is below 2" % (name, version))
    # Skip the version 1 data block, whose times are 32-bit.
    offset = header.size + timecnt * 5 + typecnt * 6 + charcnt + leap * 8 + isstd + isut
    version, isut, isstd, leap, timecnt, typecnt, charcnt = read_header(offset)
    offset += header.size
    if leap != 0:
        fail("%s: leapcnt is %d, expected 0" % (name, leap))
    if typecnt < 1:
        fail("%s: typecnt is 0" % name)

    times = struct.unpack_from(">%dq" % timecnt, data, offset)
    offset += timecnt * 8
    indices = struct.unpack_from(">%dB" % timecnt, data, offset)
    offset += timecnt
    records = []
    for _ in range(typecnt):
        records.append(struct.unpack_from(">lBB", data, offset))
        offset += 6
    chars = data[offset:offset + charcnt]
    offset += charcnt + isstd + isut

    types = []
    for utoff, isdst, desigidx in records:
        end = chars.find(b"\0", desigidx)
        if end < 0:
            fail("%s: unterminated designation at index %d" % (name, desigidx))
        types.append((utoff, isdst, chars[desigidx:end].decode("ascii")))
    for index in indices:
        if index >= typecnt:
            fail("%s: transition type index %d >= typecnt %d" % (name, index, typecnt))

    rest = data[offset:]
    if len(rest) < 2 or rest[:1] != b"\n" or rest[-1:] != b"\n":
        fail("%s: malformed footer" % name)
    footer = rest[1:-1].decode("ascii")
    if "\n" in footer:
        fail("%s: footer holds a newline" % name)
    return types, list(zip(times, indices)), footer


def check_zone(name, types, transitions, footer):
    for utoff, isdst, abbr in types:
        if not DESIGNATION.match(abbr):
            fail("%s: designation %r has a character outside [A-Za-z0-9+-]" % (name, abbr))
        if abs(utoff) >= MAX_ABS_UTOFF:
            fail("%s: |utoff| %d is not below %d" % (name, utoff, MAX_ABS_UTOFF))
        if isdst not in (0, 1):
            fail("%s: isdst %d is not 0 or 1" % (name, isdst))
    for (earlier, _), (later, _) in zip(transitions, transitions[1:]):
        if later - earlier <= MIN_TRANSITION_GAP:
            fail("%s: transitions at %d and %d are %d s apart, not more than %d"
                 % (name, earlier, later, later - earlier, MIN_TRANSITION_GAP))
    if "|" in footer or ";" in footer or '"' in footer or "\\" in footer:
        fail("%s: footer %r holds a separator or quote character" % (name, footer))
    if footer == "":
        return
    match = FOOTER.match(footer)
    if not match:
        fail("%s: footer %r is outside the accepted TZ grammar" % (name, footer))
    if match.group("dst") and not match.group("start"):
        fail("%s: footer %r names a DST designation without a ,start,end rule" % (name, footer))
    for part in ("start", "end"):
        rule = match.group(part)
        if rule is not None and not re.match("^%s$" % FOOTER_RULE, rule):
            fail("%s: footer %r rule %r is not an Mm.w.d date" % (name, footer, rule))
        if rule is not None:
            month, week, day = (int(v) for v in re.match(r"M(\d+)\.(\d)\.(\d)", rule).groups())
            if not (1 <= month <= 12 and 1 <= week <= 5 and 0 <= day <= 6):
                fail("%s: footer %r rule %r is out of range" % (name, footer, rule))
    for part in ("stdoff", "dstoff"):
        value = match.group(part)
        if value is not None:
            fields = [int(v) for v in value.lstrip("+-").split(":")]
            seconds = fields[0] * 3600 + (fields[1] * 60 if len(fields) > 1 else 0) + (fields[2] if len(fields) > 2 else 0)
            if fields[0] > 24 or seconds >= MAX_ABS_UTOFF:
                fail("%s: footer %r offset %r is out of range" % (name, footer, value))


def load_zones(out):
    by_bytes = {}
    for directory, _, files in os.walk(out):
        for file in files:
            path = os.path.join(directory, file)
            name = os.path.relpath(path, out).replace(os.sep, "/")
            if '"' in name or "\\" in name:
                fail("zone name %r holds a quote or backslash" % name)
            with open(path, "rb") as f:
                by_bytes.setdefault(f.read(), []).append(name)

    names = sorted(n for group in by_bytes.values() for n in group)
    folded = {}
    for name in names:
        key = name.lower()
        if key in folded:
            fail("zone names %r and %r are equal ignoring case" % (folded[key], name))
        folded[key] = name

    groups = []
    for data, members in by_bytes.items():
        members = sorted(members)
        types, transitions, footer = parse_tzif(members[0], data)
        check_zone(members[0], types, transitions, footer)
        groups.append((members, types, transitions, footer))
    groups.sort(key=lambda g: g[0][0])
    return names, groups


def bucket_of(name):
    first = name[0].lower()
    if not ("a" <= first <= "z"):
        fail("zone name %r does not start with a letter" % name)
    return first


def zone_string(types, transitions, footer):
    type_part = ";".join("%d,%d,%s" % t for t in types)
    transition_part = ";".join("%d,%d" % t for t in transitions)
    return "%s|%s|%s" % (type_part, transition_part, footer)


def emit(names, groups):
    lines = []
    add = lines.append
    add("' GENERATED by tools/tzdb/gen_timezones_data.py from third_party/tzdb (IANA tzdb %s)." % RELEASE)
    add("' Do not edit: `sh scripts/check-generated.sh` fails on any hand change.")
    add("'")
    add("' Each zone string is `types|transitions|footer` (plan-135-A section 4.3):")
    add("'   types       `utoff,isdst,abbr` records joined by `;`, in TZif order")
    add("'   transitions `unixSeconds,typeIndex` records joined by `;`, ascending")
    add("'   footer      the RFC 8536 footer TZ string, possibly empty")
    add("' Names that compile to identical TZif bytes share one zone string.")
    add("IMPORT strings")
    add("")
    add("FUNC tzdbVersion() AS String")
    add('  RETURN "%s"' % RELEASE)
    add("END FUNC")
    add("")
    add("FUNC zoneNames() AS List OF String")
    add("  RETURN [%s]" % ", ".join('"%s"' % n for n in names))
    add("END FUNC")
    add("")

    buckets = sorted({bucket_of(n) for n in names})

    def dispatcher(func, comment, prefix):
        add("' %s" % comment)
        add("FUNC %s(name AS String) AS String" % func)
        add('  IF len(name) = 0 THEN RETURN ""')
        add("  LET key AS String = strings::lower(name)")
        add("  MATCH strings::left(key, 1)")
        for b in buckets:
            add('    CASE "%s" : RETURN %s%s(key)' % (b, prefix, b.upper()))
        add('    CASE ELSE : RETURN ""')
        add("  END MATCH")
        add("END FUNC")
        add("")

    dispatcher("zoneData", 'The zone string for a name, ignoring case; "" for an unknown name.', "zones")
    dispatcher("canonicalName", 'The tzdb spelling of a name, ignoring case; "" for an unknown name.', "names")

    for b in buckets:
        add("PRIVATE FUNC zones%s(key AS String) AS String" % b.upper())
        add("  MATCH key")
        for index, (members, _, _, _) in enumerate(groups):
            inside = [m.lower() for m in members if bucket_of(m) == b]
            if inside:
                add("    CASE %s : RETURN zone%d()" % (", ".join('"%s"' % m for m in inside), index))
        add('    CASE ELSE : RETURN ""')
        add("  END MATCH")
        add("END FUNC")
        add("")

    for b in buckets:
        add("PRIVATE FUNC names%s(key AS String) AS String" % b.upper())
        add("  MATCH key")
        for n in names:
            if bucket_of(n) == b:
                add('    CASE "%s" : RETURN "%s"' % (n.lower(), n))
        add('    CASE ELSE : RETURN ""')
        add("  END MATCH")
        add("END FUNC")
        add("")

    for index, (members, types, transitions, footer) in enumerate(groups):
        add("' %s" % ", ".join(members))
        add("PRIVATE FUNC zone%d() AS String" % index)
        add('  RETURN "%s"' % zone_string(types, transitions, footer))
        add("END FUNC")
        add("")

    return "\n".join(lines).rstrip("\n") + "\n"


def main():
    verify_checksums()
    with tempfile.TemporaryDirectory() as work:
        out = compile_zones(work)
        names, groups = load_zones(out)
    text = emit(names, groups)
    sys.stdout.buffer.write(text.encode("ascii"))
    sys.stderr.write("names %d distinct %d transitions %d types %d footers %d\n" % (
        len(names),
        len(groups),
        sum(len(g[2]) for g in groups),
        sum(len(g[1]) for g in groups),
        len({g[3] for g in groups}),
    ))


if __name__ == "__main__":
    main()
