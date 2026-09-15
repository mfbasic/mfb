#!/usr/bin/env python3
"""Python judge for the compress oracle: stdlib `zlib` / `gzip` only.

Usage: oracle.py <mode> <job-path>

Reads the job `gen.py` wrote and prints one `case <index> <fields...>` line per case,
in the same shape the MFB probe prints, so `run.sh` can diff the two line for line.
"""

import struct
import sys
import zlib


def read_job(path):
    with open(path, "rb") as f:
        blob = f.read()
    (count,) = struct.unpack_from("<I", blob, 0)
    offset = 4 + 8 * count
    for i in range(count):
        n, aux = struct.unpack_from("<II", blob, 4 + 8 * i)
        yield i, blob[offset : offset + n], aux
        offset += n


def crc32(index, data, split):
    whole = zlib.crc32(data)
    chained = zlib.crc32(data[split:], zlib.crc32(data[:split]))
    return f"case {index} {whole} {chained}"


MODES = {
    "crc32": crc32,
}


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in MODES:
        sys.exit(f"usage: oracle.py <{'|'.join(MODES)}> <job-path>")
    judge = MODES[sys.argv[1]]
    for index, data, aux in read_job(sys.argv[2]):
        print(judge(index, data, aux))


if __name__ == "__main__":
    main()
