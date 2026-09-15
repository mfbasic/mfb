"""Hand-built streams for the zlib behaviour probe (plan-137-B Phase 1).

`cases()` returns `(name, format, bytes)` triples, `format` one of "raw", "zlib", "gzip".
Every stream is built here bit by bit — a DEFLATE writer independent of any zlib — so the
probe measures how a zlib *decoder* treats inputs no zlib encoder would produce: code sets
at the edges of `inflate_table`'s validity rules, header flags, and bad trailers.

DEFLATE packing (RFC 1951 §3.1.1): fields are written least-significant bit first; Huffman
codes are written most-significant bit first, i.e. bit-reversed into the stream.
"""

import gzip
import heapq
import struct
import zlib

PAYLOAD = b"compress probe: hello hello hello, the quick brown fox. " * 4


class BitWriter:
    def __init__(self):
        self.out = bytearray()
        self.acc = 0
        self.n = 0

    def bits(self, value, count):
        """A `count`-bit field, least significant bit first."""
        self.acc |= value << self.n
        self.n += count
        while self.n >= 8:
            self.out.append(self.acc & 255)
            self.acc >>= 8
            self.n -= 8

    def code(self, code, length):
        """A Huffman code: its bits most significant first."""
        rev = 0
        for i in range(length):
            rev |= ((code >> i) & 1) << (length - 1 - i)
        self.bits(rev, length)

    def finish(self):
        if self.n:
            self.out.append(self.acc & 255)
            self.acc = 0
            self.n = 0
        return bytes(self.out)


def canonical(lengths):
    """RFC 1951 §3.2.2: code for each symbol from its length (0 = unused)."""
    maxlen = max(lengths) if lengths else 0
    bl_count = [0] * (maxlen + 1)
    for l in lengths:
        if l:
            bl_count[l] += 1
    code = 0
    next_code = [0] * (maxlen + 2)
    for bits in range(1, maxlen + 1):
        code = (code + bl_count[bits - 1]) << 1 if bits > 1 else 0
        next_code[bits] = code
    codes = [0] * len(lengths)
    for sym, l in enumerate(lengths):
        if l:
            codes[sym] = next_code[l]
            next_code[l] += 1
    return codes


def complete_lengths(symbols, total):
    """A complete Huffman code over `symbols` (equal weights), as a length list of `total`."""
    symbols = sorted(set(symbols))
    if len(symbols) == 1:
        # A complete code needs two leaves; give the extra leaf to an unused symbol.
        spare = next(s for s in range(total) if s not in symbols)
        symbols = sorted(symbols + [spare])
    heap = [(1, i, [s]) for i, s in enumerate(symbols)]
    heapq.heapify(heap)
    depth = {s: 0 for s in symbols}
    tiebreak = len(heap)
    while len(heap) > 1:
        w1, _, a = heapq.heappop(heap)
        w2, _, b = heapq.heappop(heap)
        for s in a + b:
            depth[s] += 1
        tiebreak += 1
        heapq.heappush(heap, (w1 + w2, tiebreak, a + b))
    lengths = [0] * total
    for s, d in depth.items():
        lengths[s] = d
    return lengths


CL_ORDER = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]


def dynamic_block(lit_lengths, dist_lengths, symbols, final=True):
    """One dynamic-Huffman block.

    `lit_lengths` (257..286 entries) and `dist_lengths` (1..30 entries) are written as-is,
    valid or not — that is the point. `symbols` is a list of ("lit", byte), ("len", sym,
    extra_bits, extra_count), ("dist", sym, extra_bits, extra_count), ("raw", value, count)
    or ("eob",). The code-length alphabet is written with a complete code and no repeat
    codes, so only the two sets under test can be invalid.
    """
    w = BitWriter()
    w.bits(1 if final else 0, 1)
    w.bits(2, 2)
    hlit, hdist = len(lit_lengths), len(dist_lengths)
    seq = list(lit_lengths) + list(dist_lengths)
    cl_lengths = complete_lengths(set(seq), 19)
    cl_codes = canonical(cl_lengths)
    hclen = 19
    while hclen > 4 and cl_lengths[CL_ORDER[hclen - 1]] == 0:
        hclen -= 1
    w.bits(hlit - 257, 5)
    w.bits(hdist - 1, 5)
    w.bits(hclen - 4, 4)
    for i in range(hclen):
        w.bits(cl_lengths[CL_ORDER[i]], 3)
    for l in seq:
        w.code(cl_codes[l], cl_lengths[l])
    lit_codes = canonical(lit_lengths)
    dist_codes = canonical(dist_lengths)
    for s in symbols:
        kind = s[0]
        if kind == "lit":
            w.code(lit_codes[s[1]], lit_lengths[s[1]])
        elif kind == "len":
            w.code(lit_codes[s[1]], lit_lengths[s[1]])
            w.bits(s[2], s[3])
        elif kind == "dist":
            w.code(dist_codes[s[1]], dist_lengths[s[1]])
            w.bits(s[2], s[3])
        elif kind == "raw":
            w.bits(s[1], s[2])
        elif kind == "eob":
            w.code(lit_codes[256], lit_lengths[256])
    return w.finish()


def lit_set(used, hlit=286):
    return complete_lengths(used, hlit)


def zlib_wrap(raw, data, flg_extra=0, dictid=None, adler=None):
    cmf = 0x78
    flg = flg_extra & 0xE0
    flg |= 31 - ((cmf * 256 + flg) % 31)
    out = bytes([cmf, flg])
    if dictid is not None:
        out += struct.pack(">I", dictid)
    return out + raw + struct.pack(">I", zlib.adler32(data) if adler is None else adler)


def gzip_member(raw, data, flags=0, hcrc_delta=0, crc=None, isize=None):
    header = bytes([0x1F, 0x8B, 8, flags]) + struct.pack("<I", 0) + bytes([0, 255])
    if flags & 0x02:
        hcrc = (zlib.crc32(header) & 0xFFFF) ^ hcrc_delta
        header += struct.pack("<H", hcrc)
    trailer = struct.pack("<II", zlib.crc32(data) if crc is None else crc,
                          (len(data) & 0xFFFFFFFF) if isize is None else isize)
    return header + raw + trailer


def raw_deflate(data, level=6):
    c = zlib.compressobj(level, zlib.DEFLATED, -15)
    return c.compress(data) + c.flush()


def cases():
    out = []
    raw = raw_deflate(PAYLOAD)
    z = zlib.compress(PAYLOAD, 6)
    g = gzip.compress(PAYLOAD, 6, mtime=0)

    # Trailing bytes after a complete stream.
    out.append(("raw-valid", "raw", raw))
    out.append(("zlib-valid", "zlib", z))
    out.append(("gzip-valid", "gzip", g))
    for n in (1, 7, 1000):
        junk = bytes((0x5A + i) % 251 for i in range(n))
        out.append((f"raw-trailing-{n}", "raw", raw + junk))
        out.append((f"zlib-trailing-{n}", "zlib", z + junk))
        out.append((f"gzip-trailing-{n}", "gzip", g + junk))
    out.append(("gzip-two-members", "gzip", g + gzip.compress(b"second member", 6, mtime=0)))
    out.append(("gzip-member-then-magic-garbage", "gzip", g + b"\x1f\x8b\x00\x00junk"))

    # Checksums and header flags.
    out.append(("zlib-bad-adler", "zlib", zlib_wrap(raw, PAYLOAD, adler=zlib.adler32(PAYLOAD) ^ 1)))
    out.append(("zlib-fdict", "zlib", zlib_wrap(raw, PAYLOAD, flg_extra=0x20, dictid=0x12345678)))
    out.append(("zlib-cinfo-8", "zlib", bytes([0x88, (31 - (0x88 * 256) % 31) % 31]) + raw
                + struct.pack(">I", zlib.adler32(PAYLOAD))))
    out.append(("gzip-fhcrc-good", "gzip", gzip_member(raw, PAYLOAD, flags=0x02)))
    out.append(("gzip-fhcrc-bad", "gzip", gzip_member(raw, PAYLOAD, flags=0x02, hcrc_delta=1)))
    out.append(("gzip-bad-crc", "gzip", gzip_member(raw, PAYLOAD, crc=zlib.crc32(PAYLOAD) ^ 1)))
    out.append(("gzip-bad-isize", "gzip", gzip_member(raw, PAYLOAD, isize=len(PAYLOAD) + 1)))
    out.append(("gzip-reserved-flag", "gzip", gzip_member(raw, PAYLOAD, flags=0x20)))

    # Code sets at the edges of inflate_table's rules.
    abc = [ord("a"), ord("b"), ord("c")]
    lits = lit_set(abc + [256, 257])
    # One distance code of length 1 (incomplete, max == 1): "abc" then length 3 at distance 1.
    out.append(("raw-one-distance-code-used", "raw",
                dynamic_block(lits, [1], [("lit", abc[0]), ("lit", abc[1]), ("lit", abc[2]),
                                          ("len", 257, 0, 0), ("dist", 0, 0, 0), ("eob",)])))
    out.append(("raw-one-distance-code-unused", "raw",
                dynamic_block(lits, [1], [("lit", abc[0]), ("eob",)])))
    # No distance codes at all (max == 0), used and unused.
    out.append(("raw-no-distance-codes-unused", "raw",
                dynamic_block(lits, [0], [("lit", abc[0]), ("lit", abc[1]), ("eob",)])))
    out.append(("raw-no-distance-codes-used", "raw",
                dynamic_block(lits, [0], [("lit", abc[0]), ("len", 257, 0, 0), ("raw", 0, 8), ("eob",)])))
    # Two distance codes of length 1 plus one of length 2: over-subscribed distance set.
    out.append(("raw-oversubscribed-distances", "raw",
                dynamic_block(lits, [1, 1, 2], [("lit", abc[0]), ("eob",)])))
    # Literal/length sets: incomplete with max > 1, over-subscribed, single length-1 code,
    # and missing end-of-block.
    incomplete = [0] * 286
    for s in (abc[0], abc[1], 256):
        incomplete[s] = 2
    out.append(("raw-incomplete-literal-set", "raw",
                dynamic_block(incomplete, [0], [("lit", abc[0]), ("eob",)])))
    over = [0] * 286
    for s in (abc[0], abc[1], 256):
        over[s] = 1
    out.append(("raw-oversubscribed-literal-set", "raw",
                dynamic_block(over, [0], [("lit", abc[0]), ("eob",)])))
    single = [0] * 257
    single[256] = 1
    out.append(("raw-single-literal-code", "raw", dynamic_block(single, [0], [("eob",)])))
    no_eob = lit_set(abc, 286)
    no_eob[256] = 0
    out.append(("raw-missing-end-of-block", "raw", dynamic_block(no_eob, [0], [("lit", abc[0])])))
    # Distance reaching before the start of output: length 3 at distance 2 after one byte.
    out.append(("raw-distance-too-far", "raw",
                dynamic_block(lits, [0, 1, 1], [("lit", abc[0]), ("len", 257, 0, 0), ("dist", 1, 0, 0), ("eob",)])))
    # Stored block with LEN != ~NLEN, and reserved BTYPE 3.
    out.append(("raw-stored-bad-nlen", "raw", bytes([1, 3, 0, 0xFC, 0xFE]) + b"abc"))
    w = BitWriter()
    w.bits(1, 1)
    w.bits(3, 2)
    out.append(("raw-reserved-btype", "raw", w.finish() + b"\x00\x00"))
    return out
