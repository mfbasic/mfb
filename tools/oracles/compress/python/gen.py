#!/usr/bin/env python3
"""Write a job file for one oracle mode.

Usage: gen.py <mode> <job-path>

A job is what the MFB probe and every judge read; none of them derives its inputs
from another's output. Layout (all integers little-endian u32):

    count
    (length, aux) * count      -- `aux` is per mode: the split point for crc32
    bytes of case 0, case 1, ...

The corpus is seeded, so a failing case reproduces. Each mode prints its case count
on stdout; `run.sh` compares that with the count it declares, never with what a
subject printed.
"""

import gzip
import random
import struct
import sys
import zlib

SEED = 137


def crc32_cases(rng):
    # Every length 0..17 covers each slicing-by-8 tail with and without an
    # eight-byte step before it; then random lengths up to 1 MiB.
    lengths = list(range(18)) + [rng.randrange(0, 1024 * 1024 + 1) for _ in range(100)]
    cases = []
    for n in lengths:
        data = rng.randbytes(n)
        cases.append((data, rng.randrange(0, n + 1)))
    return cases


def corpora(rng):
    """Three payloads with different statistics: text, incompressible, and a mix with long runs."""
    text = b"".join(f"record {i}: the quick brown fox {i % 97} jumps over {i * 7 % 1000}\n".encode()
                    for i in range(1200))
    noise = rng.randbytes(30000)
    mixed = text[:40000] + bytes(30000) + rng.randbytes(20000) + text[40000:50000] * 3
    return [text, noise, mixed]


STRATEGIES = [zlib.Z_DEFAULT_STRATEGY, zlib.Z_FILTERED, zlib.Z_HUFFMAN_ONLY, zlib.Z_RLE, zlib.Z_FIXED]


def decode_raw_cases(rng):
    """Python zlib raw DEFLATE (wbits -15) of each corpus at every level and strategy."""
    cases = []
    for payload in corpora(rng):
        for level in range(10):
            for strategy in STRATEGIES:
                c = zlib.compressobj(level, zlib.DEFLATED, -15, 8, strategy)
                cases.append((c.compress(payload) + c.flush(), 0))
    return cases


def decode_zlib_cases(rng):
    """Python zlib's zlib-wrapped output (wbits 15) of each corpus at every level and strategy."""
    cases = []
    for payload in corpora(rng):
        for level in range(10):
            for strategy in STRATEGIES:
                c = zlib.compressobj(level, zlib.DEFLATED, 15, 8, strategy)
                cases.append((c.compress(payload) + c.flush(), 0))
    return cases


def decode_gzip_cases(rng):
    """gzip (wbits 31) at every level and strategy, then multi-member files and every optional header
    field, written by hand so FEXTRA / FNAME / FCOMMENT / FHCRC appear together and apart."""
    cases = []
    texts = corpora(rng)
    for payload in texts:
        for level in range(10):
            for strategy in STRATEGIES:
                c = zlib.compressobj(level, zlib.DEFLATED, 31, 8, strategy)
                cases.append((c.compress(payload) + c.flush(), 0))
    small = texts[0][:5000]
    one = gzip.compress(small, 6, mtime=0)
    cases.append((one + gzip.compress(texts[1][:3000], 1, mtime=0), 0))
    cases.append((one + one + one, 0))
    cases.append((one + b"trailing padding that is not a member", 0))
    for flags in (0x04, 0x08, 0x10, 0x02, 0x0E, 0x1E):
        header = bytes([0x1F, 0x8B, 8, flags]) + struct.pack("<I", 0) + bytes([0, 3])
        if flags & 0x04:
            header += struct.pack("<H", 6) + b"XY\x02\x00ab"
        if flags & 0x08:
            header += b"name.txt\x00"
        if flags & 0x10:
            header += b"a comment\x00"
        if flags & 0x02:
            header += struct.pack("<H", zlib.crc32(header) & 0xFFFF)
        c = zlib.compressobj(6, zlib.DEFLATED, -15)
        body = c.compress(small) + c.flush()
        cases.append((header + body + struct.pack("<II", zlib.crc32(small), len(small)), 0))
    return cases


PROBE_FORMATS = {"raw": 0, "zlib": 1, "gzip": 2}


def probe_cases(_rng, names_path):
    """Hand-built edge streams (probe_streams.py); `aux` is the format, names go beside the job."""
    from probe_streams import cases as streams

    named = streams()
    with open(names_path, "w") as f:
        for name, fmt, _ in named:
            f.write(f"{name} {fmt}\n")
    return [(data, PROBE_FORMATS[fmt]) for _, fmt, data in named]


def mutate_cases(rng):
    """600 seeded edits of valid streams: 1-3 byte replacements, deletions or insertions of raw,
    zlib and gzip streams over three payloads and four level/strategy settings. `aux` is the
    format (0 raw, 1 zlib, 2 gzip)."""
    text = corpora(rng)[0]
    payloads = [text[:4000], rng.randbytes(1500), bytes(3000) + b"tail" * 50]
    settings = [(0, zlib.Z_DEFAULT_STRATEGY), (1, zlib.Z_DEFAULT_STRATEGY), (6, zlib.Z_DEFAULT_STRATEGY),
                (9, zlib.Z_FIXED)]
    bases = []
    for payload in payloads:
        for level, strategy in settings:
            for fmt, wbits in ((0, -15), (1, 15), (2, 31)):
                c = zlib.compressobj(level, zlib.DEFLATED, wbits, 8, strategy)
                bases.append((c.compress(payload) + c.flush(), fmt))
    cases = []
    for _ in range(600):
        data, fmt = bases[rng.randrange(len(bases))]
        edited = bytearray(data)
        for _ in range(rng.randint(1, 3)):
            kind = rng.randrange(3)
            if kind == 0 and edited:
                edited[rng.randrange(len(edited))] = rng.randrange(256)
            elif kind == 1 and edited:
                del edited[rng.randrange(len(edited))]
            else:
                edited.insert(rng.randrange(len(edited) + 1), rng.randrange(256))
        cases.append((bytes(edited), fmt))
    return cases


MODES = {
    "crc32": crc32_cases,
    "probe": probe_cases,
    "decode-raw": decode_raw_cases,
    "decode-zlib": decode_zlib_cases,
    "decode-gzip": decode_gzip_cases,
    "mutate": mutate_cases,
}


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in MODES:
        sys.exit(f"usage: gen.py <{'|'.join(MODES)}> <job-path>")
    mode, path = sys.argv[1], sys.argv[2]
    rng = random.Random(f"{SEED}-{mode}")
    cases = MODES[mode](rng, path + ".names") if mode == "probe" else MODES[mode](rng)
    with open(path, "wb") as out:
        out.write(struct.pack("<I", len(cases)))
        for data, aux in cases:
            out.write(struct.pack("<II", len(data), aux))
        for data, _ in cases:
            out.write(data)
    print(len(cases))


if __name__ == "__main__":
    main()
