"""Compression benchmark (group `compress`).

Mirrors benchmark/language/mfb/src/compress.mfb: compress::crc32 / deflate / inflate /
gzipEncode+gzipDecode against the zlib module (CPython's standard means of
doing the job; raw DEFLATE is wbits=-15, gzip wbits=31, level 6).

crc32 is a fixed function of the bytes. The deflate/inflate/gzip rows never
checksum a compressed length (zlib's bytes legitimately differ from mfb's
encoder): per repetition they add len + crc32 of the round-tripped
(decompressed) data, which matches mfb and C bit-for-bit. `inflate` decodes
zlib's own level-6 stream of the same text.

Input: the same deterministic ~128 KB word-salad text as compress.mfb /
compressbench.c, built once, untimed.
"""
import sys
import zlib

RUN = 1
now_ns = None
record = None

COMPRESS_TARGET = 131072
CRC_REPS = 64
DEFLATE_REPS = 4
INFLATE_REPS = 16
GZIP_REPS = 4
RAW_BITS = -15
GZIP_BITS = 31

WORDS = ["the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog",
         "compress", "deflate", "window", "huffman", "literal", "length",
         "distance", "block"]


def compress_input(target):
    # LCG x = (x*1103515245 + 12345) mod 2^31 picks word (x//65536) % 16; the
    # separator is "\n" when (x//1024) % 13 == 0, else " ".
    parts = []
    total = 0
    x = 12345
    while total < target:
        x = (x * 1103515245 + 12345) % 2147483648
        w = WORDS[(x // 65536) % 16]
        parts.append(w + ("\n" if (x // 1024) % 13 == 0 else " "))
        total += len(w) + 1
    return "".join(parts).encode("utf-8")


def test_compress_crc32():
    x = compress_input(COMPRESS_TARGET)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(CRC_REPS):
            acc += zlib.crc32(x)
        checksum = acc
        times.append(now_ns() - t0)
    print("test_compress_crc32 = %d" % checksum, file=sys.stderr)
    record("compress", "crc32", times)


def z_encode(x, wbits):
    c = zlib.compressobj(6, zlib.DEFLATED, wbits)
    return c.compress(x) + c.flush()


def z_decode(d, wbits):
    return zlib.decompress(d, wbits)


def round_trip(y):
    # Decompressed length + its CRC-32: independent of the compressor.
    return len(y) + zlib.crc32(y)


def test_compress_deflate():
    x = compress_input(COMPRESS_TARGET)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        outs = [z_encode(x, RAW_BITS) for _rep in range(DEFLATE_REPS)]
        times.append(now_ns() - t0)
        checksum = sum(round_trip(z_decode(d, RAW_BITS)) for d in outs)
    print("test_compress_deflate = %d" % checksum, file=sys.stderr)
    record("compress", "deflate", times)


def test_compress_inflate():
    d = z_encode(compress_input(COMPRESS_TARGET), RAW_BITS)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(INFLATE_REPS):
            acc += round_trip(z_decode(d, RAW_BITS))
        checksum = acc
        times.append(now_ns() - t0)
    print("test_compress_inflate = %d" % checksum, file=sys.stderr)
    record("compress", "inflate", times)


def test_compress_gzip():
    x = compress_input(COMPRESS_TARGET)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(GZIP_REPS):
            acc += round_trip(z_decode(z_encode(x, GZIP_BITS), GZIP_BITS))
        checksum = acc
        times.append(now_ns() - t0)
    print("test_compress_gzip = %d" % checksum, file=sys.stderr)
    record("compress", "gzip", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_compress_crc32()
    test_compress_deflate()
    test_compress_inflate()
    test_compress_gzip()
