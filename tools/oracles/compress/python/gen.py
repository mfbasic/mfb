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

import random
import struct
import sys

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


MODES = {
    "crc32": crc32_cases,
}


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in MODES:
        sys.exit(f"usage: gen.py <{'|'.join(MODES)}> <job-path>")
    mode, path = sys.argv[1], sys.argv[2]
    cases = MODES[mode](random.Random(f"{SEED}-{mode}"))
    with open(path, "wb") as out:
        out.write(struct.pack("<I", len(cases)))
        for data, aux in cases:
            out.write(struct.pack("<II", len(data), aux))
        for data, _ in cases:
            out.write(data)
    print(len(cases))


if __name__ == "__main__":
    main()
