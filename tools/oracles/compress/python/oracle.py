#!/usr/bin/env python3
"""Python judge for the compress oracle: stdlib `zlib` / `gzip` only.

Usage: oracle.py <mode> <job-path>

Reads the job `gen.py` wrote and prints one `case <index> <fields...>` line per case,
in the same shape the MFB probe prints, so `run.sh` can diff the two line for line.
"""

import gzip
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


def _outcome(fn):
    try:
        return fn()
    except Exception as e:  # zlib.error, gzip.BadGzipFile, EOFError, OSError
        return f"err {type(e).__name__}: {e}"


def probe(index, data, fmt):
    """How Python's zlib treats one probe stream: decompressobj (one stream) and, for gzip,
    gzip.decompress (every member)."""
    wbits = {0: -15, 1: 15, 2: 31}[fmt]

    def one_stream():
        d = zlib.decompressobj(wbits)
        out = d.decompress(data)
        return f"ok out={len(out)} eof={d.eof} unused={len(d.unused_data)}"

    fields = f"decompressobj[{_outcome(one_stream)}]"
    if fmt == 2:
        fields += f" gzip.decompress[{_outcome(lambda: f'ok out={len(gzip.decompress(data))}')}]"
    return f"case {index} {fields}"


def decode_raw(index, data, _aux):
    """Length and CRC-32 of Python zlib's raw DEFLATE decode, or the error."""
    try:
        d = zlib.decompressobj(-15)
        out = d.decompress(data) + d.flush()
    except zlib.error as e:
        return f"case {index} err {e}"
    return f"case {index} {len(out)} {zlib.crc32(out)}"


MODES = {
    "crc32": crc32,
    "probe": probe,
    "decode-raw": decode_raw,
}


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in MODES:
        sys.exit(f"usage: oracle.py <{'|'.join(MODES)}> <job-path>")
    judge = MODES[sys.argv[1]]
    for index, data, aux in read_job(sys.argv[2]):
        print(judge(index, data, aux))


if __name__ == "__main__":
    main()
