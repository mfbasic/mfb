"""CRC-32 benchmark (group `compress`).

Mirrors benchmark/mfb/src/compress.mfb: compress::crc32 against zlib.crc32
(CPython's standard means of doing the job). The CRC is a fixed function of the
bytes, so the checksum matches mfb and C bit-for-bit.

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


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_compress_crc32()
