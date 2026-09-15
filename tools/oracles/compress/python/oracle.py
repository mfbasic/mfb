#!/usr/bin/env python3
"""Python judge for the compress oracle: stdlib `zlib` / `gzip` only.

Usage: oracle.py <mode> <job-path>
       oracle.py <encode-mode> <job-path> <mfb-output-path>

Reads the job `gen.py` wrote and prints one `case <index> <fields...>` line per case,
in the same shape the MFB probe prints, so `run.sh` can diff the two line for line.
"""

import gzip
import struct
import subprocess
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


def decode_zlib(index, data, _aux):
    """Length and CRC-32 of Python zlib's zlib-format decode, or the error."""
    try:
        d = zlib.decompressobj(15)
        out = d.decompress(data) + d.flush()
        if not d.eof:
            raise zlib.error("incomplete stream")
    except zlib.error as e:
        return f"case {index} err {e}"
    return f"case {index} {len(out)} {zlib.crc32(out)}"


def decode_gzip(index, data, _aux):
    """Every gzip member through zlib's own gzip decoder (wbits 31), continuing while the remaining
    bytes start 1f 8b. `gzip.decompress` is not used: it accepts a wrong header CRC and reserved
    flags that zlib refuses (probe.sh)."""
    out = b""
    rest = data
    try:
        while True:
            d = zlib.decompressobj(31)
            out += d.decompress(rest) + d.flush()
            if not d.eof:
                raise zlib.error("incomplete stream")
            rest = d.unused_data
            if rest[:2] != b"\x1f\x8b":
                break
    except zlib.error as e:
        return f"case {index} err {e}"
    return f"case {index} {len(out)} {zlib.crc32(out)}"


def mutate(index, data, fmt):
    """Verdict only — `ok <len> <crc32>` or `err` — because error messages are not comparable across
    decoders. A stream that stops before its end is `err`, as `compress` refuses it."""
    try:
        if fmt == 2:
            out = b""
            rest = data
            while True:
                d = zlib.decompressobj(31)
                out += d.decompress(rest) + d.flush()
                if not d.eof:
                    raise zlib.error("incomplete stream")
                rest = d.unused_data
                if rest[:2] != b"\x1f\x8b":
                    break
        else:
            d = zlib.decompressobj(-15 if fmt == 0 else 15)
            out = d.decompress(data) + d.flush()
            if not d.eof:
                raise zlib.error("incomplete stream")
    except zlib.error:
        return f"case {index} err"
    return f"case {index} ok {len(out)} {zlib.crc32(out)}"


def encode_raw(index, data, _level, produced):
    """Decode what the MFB probe produced from this payload; it must be exactly the payload, with
    nothing after the end of the DEFLATE data."""
    try:
        d = zlib.decompressobj(-15)
        out = d.decompress(produced) + d.flush()
        if not d.eof:
            raise zlib.error("incomplete stream")
    except zlib.error as e:
        return f"case {index} err {e}"
    if out != data:
        return f"case {index} MISMATCH {len(out)} {zlib.crc32(out)}"
    if d.unused_data:
        return f"case {index} TRAILING {len(d.unused_data)}"
    return f"case {index} {len(data)} {zlib.crc32(data)}"


def encode_zlib(index, data, level, produced):
    """zlib's decode of the produced zlib stream must be the payload, and the two header bytes must
    be the ones zlib's own compressor writes at this level (`CMF`, `FLEVEL`, `FCHECK`)."""
    try:
        d = zlib.decompressobj(15)
        out = d.decompress(produced) + d.flush()
        if not d.eof:
            raise zlib.error("incomplete stream")
    except zlib.error as e:
        return f"case {index} err {e}"
    if out != data:
        return f"case {index} MISMATCH {len(out)} {zlib.crc32(out)}"
    if d.unused_data:
        return f"case {index} TRAILING {len(d.unused_data)}"
    want = zlib.compress(b"", level)[:2]
    if produced[:2] != want:
        return f"case {index} HEADER {produced[:2].hex()} want {want.hex()}"
    return f"case {index} {len(data)} {zlib.crc32(data)}"


def encode_gzip(index, data, level, produced):
    """zlib's gzip decode (wbits 31) of the produced member must be the payload; the header must be
    `1f 8b 08`, `FLG 0`, `MTIME 0`, the `XFL` zlib's gzip wrapper writes at this level, and `OS 255`;
    and the host's `gzip -t` must accept it."""
    try:
        d = zlib.decompressobj(31)
        out = d.decompress(produced) + d.flush()
        if not d.eof:
            raise zlib.error("incomplete stream")
    except zlib.error as e:
        return f"case {index} err {e}"
    if out != data:
        return f"case {index} MISMATCH {len(out)} {zlib.crc32(out)}"
    if d.unused_data:
        return f"case {index} TRAILING {len(d.unused_data)}"
    c = zlib.compressobj(level, zlib.DEFLATED, 31)
    want = (c.compress(b"") + c.flush())[:9] + b"\xff"
    if produced[:10] != want:
        return f"case {index} HEADER {produced[:10].hex()} want {want.hex()}"
    tested = subprocess.run(["gzip", "-t"], input=produced, capture_output=True)
    if tested.returncode != 0:
        return f"case {index} GZIP-T {tested.returncode} {tested.stderr.decode(errors='replace').strip()}"
    return f"case {index} {len(data)} {zlib.crc32(data)}"


# Encode modes judge the MFB probe's output file (a job of the same layout) against the payloads.
ENCODE_MODES = {
    "encode-raw": encode_raw,
    "encode-zlib": encode_zlib,
    "encode-gzip": encode_gzip,
}

MODES = {
    "crc32": crc32,
    "probe": probe,
    "decode-raw": decode_raw,
    "decode-zlib": decode_zlib,
    "decode-gzip": decode_gzip,
    "mutate": mutate,
}


def main():
    if len(sys.argv) == 4 and sys.argv[1] in ENCODE_MODES:
        judge = ENCODE_MODES[sys.argv[1]]
        produced = [data for _, data, _ in read_job(sys.argv[3])]
        for index, data, aux in read_job(sys.argv[2]):
            print(judge(index, data, aux, produced[index]))
        return
    if len(sys.argv) != 3 or sys.argv[1] not in MODES:
        sys.exit(f"usage: oracle.py <{'|'.join(MODES)}> <job-path>")
    judge = MODES[sys.argv[1]]
    for index, data, aux in read_job(sys.argv[2]):
        print(judge(index, data, aux))


if __name__ == "__main__":
    main()
