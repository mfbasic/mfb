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


# --- plan-137-E per-block audit: re-decode the produced DEFLATE data bit by bit and check that no
# dynamic block is larger than the fixed-Huffman or stored encoding of the same block ---

FIXED_LIT_LENS = [8] * 144 + [9] * 112 + [7] * 24 + [8] * 8
LEN_BASE, LEN_EXTRA = [], []
_base = 3
for _i in range(28):
    _extra = (_i - 4) // 4 if _i >= 8 else 0
    LEN_BASE.append(_base)
    LEN_EXTRA.append(_extra)
    _base += 1 << _extra
LEN_BASE.append(258)
LEN_EXTRA.append(0)
DIST_BASE, DIST_EXTRA = [], []
_base = 1
for _i in range(30):
    _extra = (_i - 2) // 2 if _i >= 4 else 0
    DIST_BASE.append(_base)
    DIST_EXTRA.append(_extra)
    _base += 1 << _extra
CL_ORDER = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]
AUDIT = {"stored": 0, "fixed": 0, "dynamic": 0, "max_lit_len": 0, "max_dist_len": 0, "limit_hit_cases": 0}


class BitReader:
    def __init__(self, data):
        self.data, self.pos = data, 0

    def get(self, n):
        v = 0
        for i in range(n):
            v |= ((self.data[self.pos >> 3] >> (self.pos & 7)) & 1) << i
            self.pos += 1
        return v


def canonical(lengths):
    """{(length, code): symbol} for MSB-first matching, per RFC 1951 3.2.2."""
    bl_count = [0] * 16
    for l in lengths:
        if l:
            bl_count[l] += 1
    code, next_code = 0, [0] * 16
    for bits in range(1, 16):
        code = (code + bl_count[bits - 1]) << 1
        next_code[bits] = code
    table = {}
    for sym, l in enumerate(lengths):
        if l:
            table[(l, next_code[l])] = sym
            next_code[l] += 1
    return table


def read_symbol(br, table):
    code = length = 0
    while length < 16:
        code = (code << 1) | br.get(1)
        length += 1
        sym = table.get((length, code))
        if sym is not None:
            return sym
    raise ValueError("invalid code")


FIXED_LIT_TABLE = canonical(FIXED_LIT_LENS)
FIXED_DIST_TABLE = canonical([5] * 30)


def audit_blocks(deflate):
    """Violations (strings) for every dynamic block larger than its fixed or stored encoding."""
    br = BitReader(deflate)
    violations = []
    block = 0
    case_max_lit = 0
    while True:
        start = br.pos
        bfinal, btype = br.get(1), br.get(2)
        if btype == 0:
            br.pos = (br.pos + 7) & ~7
            length = br.get(16)
            br.get(16)
            br.pos += 8 * length
            AUDIT["stored"] += 1
        else:
            if btype == 2:
                hlit, hdist, hclen = br.get(5) + 257, br.get(5) + 1, br.get(4) + 4
                cl = [0] * 19
                for i in range(hclen):
                    cl[CL_ORDER[i]] = br.get(3)
                cl_table = canonical(cl)
                lengths = []
                while len(lengths) < hlit + hdist:
                    s = read_symbol(br, cl_table)
                    if s < 16:
                        lengths.append(s)
                    elif s == 16:
                        lengths += [lengths[-1]] * (3 + br.get(2))
                    elif s == 17:
                        lengths += [0] * (3 + br.get(3))
                    else:
                        lengths += [0] * (11 + br.get(7))
                lit_lens, dist_lens = lengths[:hlit], lengths[hlit:]
                AUDIT["max_lit_len"] = max(AUDIT["max_lit_len"], max(lit_lens))
                AUDIT["max_dist_len"] = max(AUDIT["max_dist_len"], max(dist_lens))
                case_max_lit = max(case_max_lit, max(lit_lens), max(dist_lens))
                lit_table, dist_table = canonical(lit_lens), canonical(dist_lens)
                AUDIT["dynamic"] += 1
            else:
                lit_table, dist_table = FIXED_LIT_TABLE, FIXED_DIST_TABLE
                AUDIT["fixed"] += 1
            fixed_bits = 3
            produced = 0
            while True:
                sym = read_symbol(br, lit_table)
                fixed_bits += FIXED_LIT_LENS[sym]
                if sym < 256:
                    produced += 1
                elif sym == 256:
                    break
                else:
                    li = sym - 257
                    produced += LEN_BASE[li] + br.get(LEN_EXTRA[li])
                    dsym = read_symbol(br, dist_table)
                    br.get(DIST_EXTRA[dsym])
                    fixed_bits += LEN_EXTRA[li] + 5 + DIST_EXTRA[dsym]
            if btype == 2:
                actual = br.pos - start
                chunks = max(1, -(-produced // 65535))
                pad = (8 - (start + 3) % 8) % 8
                stored_bits = chunks * 35 + pad + (chunks - 1) * 5 + 8 * produced
                if actual > fixed_bits or actual > stored_bits:
                    violations.append(f"block {block}: dynamic {actual} bits, fixed {fixed_bits}, stored {stored_bits}")
        block += 1
        if bfinal:
            break
    if case_max_lit == 15:
        AUDIT["limit_hit_cases"] += 1
    return violations


def audited(index, data, deflate):
    violations = audit_blocks(deflate)
    if violations:
        return f"case {index} BLOCKCOST {violations[0]}"
    return f"case {index} {len(data)} {zlib.crc32(data)}"


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
    return audited(index, data, produced)


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
    return audited(index, data, produced[2:])


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
    return audited(index, data, produced[10:])


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
        print(f"block audit ({sys.argv[1]}): {AUDIT['stored']} stored, {AUDIT['fixed']} fixed, {AUDIT['dynamic']} dynamic;"
              f" longest literal/length code {AUDIT['max_lit_len']} bits, distance {AUDIT['max_dist_len']} bits;"
              f" {AUDIT['limit_hit_cases']} case(s) with a 15-bit code", file=sys.stderr)
        return
    if len(sys.argv) != 3 or sys.argv[1] not in MODES:
        sys.exit(f"usage: oracle.py <{'|'.join(MODES)}> <job-path>")
    judge = MODES[sys.argv[1]]
    for index, data, aux in read_job(sys.argv[2]):
        print(judge(index, data, aux))


if __name__ == "__main__":
    main()
