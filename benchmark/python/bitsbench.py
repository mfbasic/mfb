"""Bits-group benchmark (bits:: package surface).

Mirrors benchmark/mfb/src/bits.mfb test_bits_ops exactly: one coverage row that
folds a 64-bit hash with band/bor/bxor/bnot/sl/sr/sra/rl32/rr32/rl64/rr64/clz/
ctz/popCount/bswap16/bswap32/bswap64. Python ints are unbounded, so every op is
masked back to 64 bits; the accumulator is folded with XOR (never +) so the
workload never overflows. mfb Integer is signed 64-bit, so the printed checksum
is reinterpreted as a signed two's-complement value to line up with mfb.
"""
import sys

RUN = 1
now_ns = None
record = None

MASK64 = (1 << 64) - 1
MASK32 = (1 << 32) - 1
MASK16 = (1 << 16) - 1


def _band(a, b):
    return (a & b) & MASK64


def _bor(a, b):
    return (a | b) & MASK64


def _bxor(a, b):
    return (a ^ b) & MASK64


def _bnot(x):
    return (~x) & MASK64


def _sl(x, n):
    return (x << n) & MASK64


def _sr(x, n):
    # logical right shift of the 64-bit unsigned value
    return (x & MASK64) >> n


def _sra(x, n):
    # arithmetic right shift: reinterpret as signed, shift, mask back to 64 bits
    v = x & MASK64
    if v >> 63:
        v -= (1 << 64)
    return (v >> n) & MASK64


def _rl32(x, n):
    v = x & MASK32
    n &= 31
    return ((v << n) | (v >> (32 - n))) & MASK32 if n else v


def _rr32(x, n):
    v = x & MASK32
    n &= 31
    return ((v >> n) | (v << (32 - n))) & MASK32 if n else v


def _rl64(x, n):
    v = x & MASK64
    n &= 63
    return ((v << n) | (v >> (64 - n))) & MASK64 if n else v


def _rr64(x, n):
    v = x & MASK64
    n &= 63
    return ((v >> n) | (v << (64 - n))) & MASK64 if n else v


def _clz(x):
    v = x & MASK64
    if v == 0:
        return 64            # mfb clz(0) = 64
    return 63 - v.bit_length() + 1


def _ctz(x):
    v = x & MASK64
    if v == 0:
        return 64            # mfb ctz(0) = 64
    return (v & -v).bit_length() - 1


def _popcount(x):
    return bin(x & MASK64).count("1")


def _bswap16(x):
    v = x & MASK16
    return ((v & 0xFF) << 8) | ((v >> 8) & 0xFF)


def _bswap32(x):
    v = x & MASK32
    return (((v & 0x000000FF) << 24) | ((v & 0x0000FF00) << 8)
            | ((v & 0x00FF0000) >> 8) | ((v & 0xFF000000) >> 24)) & MASK32


def _bswap64(x):
    v = x & MASK64
    r = 0
    for _ in range(8):
        r = (r << 8) | (v & 0xFF)
        v >>= 8
    return r & MASK64


def _to_signed64(x):
    v = x & MASK64
    return v - (1 << 64) if v >> 63 else v


def test_bits_ops():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        h = 2166136261
        for i in range(200000):
            x = i
            h = _bxor(h, _bor(x, _sl(x, 3)))
            h = _band(h, 1152921504606846975)
            h = _rl64(h, 7)
            h = _rr64(h, 3)
            h = _bxor(h, _sr(h, 11))
            h = _bxor(h, _sra(h, 2))
            h = _bxor(h, _bnot(x))
            acc = _bxor(acc, h)
            acc = _bxor(acc, _rl32(h, 5))
            acc = _bxor(acc, _rr32(h, 9))
            acc = _bxor(acc, _bswap16(x))
            acc = _bxor(acc, _bswap32(x))
            acc = _bxor(acc, _bswap64(h))
            acc = _bxor(acc, _popcount(h))
            acc = _bxor(acc, _clz(x))
            acc = _bxor(acc, _ctz(x))
        checksum = _to_signed64(acc)
        times.append(now_ns() - t0)
    print("bits_ops = %d" % checksum, file=sys.stderr)
    record("bits", "ops", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_bits_ops()
