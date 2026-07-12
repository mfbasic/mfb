"""Math-group coverage benchmarks (math:: package surface).

Mirrors benchmark/mfb/src/math.mfb functions test_math_float, test_math_int,
test_math_simd. The 12 individual libm-severed transcendental kernels
(sin, cos, ... sqrt) deliberately stay in main.py -- they are the historical
per-kernel rows and share the _math_kernel helper there.

Fixed (test_math_fixed) is intentionally omitted: Fixed is an mfb-only type.
"""
import math
import sys
from math import (acos, asin, atan, atan2, cos, exp, log, log10,
                  pow as mpow, sin, sqrt, tan)

RUN = 1
now_ns = None
record = None

MASK64 = (1 << 64) - 1


def _round_half_away(x):
    # mfb math::round is round-half-away-from-zero; Python round() is banker's.
    if x >= 0.0:
        return math.floor(x + 0.5)
    return math.ceil(x - 0.5)


def _clampf(v, lo, hi):
    if v < lo:
        return lo
    if v > hi:
        return hi
    return v


def _clampi(v, lo, hi):
    if v < lo:
        return lo
    if v > hi:
        return hi
    return v


# Float: abs, floor, ceil, round, min, max, clamp over a double loop.
def test_math_float():
    times = []
    checksum = 0.0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0.0
        for i in range(200000):
            v = float(i) * 0.5 - 50000.0
            acc += abs(v)
            acc += float(math.floor(v * 0.001))
            acc += float(math.ceil(v * 0.001))
            acc += float(_round_half_away(v * 0.001))
            acc += min(v, 0.0)
            acc += max(v, 0.0)
            acc += _clampf(v, -10.0, 10.0)
        checksum = acc
        times.append(now_ns() - t0)
    print("math_float = %.3f" % checksum, file=sys.stderr)
    record("math", "float", times)


# Integer: abs, min, max, clamp + a seeded deterministic LCG (stands in for
# mfb's PCG64 math::rand; the streams differ across languages by design, so the
# checksum only has to be reproducible run-to-run, not identical to mfb).
class _Lcg:
    def __init__(self, seed):
        self.state = seed & MASK64

    def rand(self, lo, hi):
        # SplitMix/PCG-style constants; deterministic 64-bit LCG.
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) & MASK64
        span = hi - lo + 1
        return lo + ((self.state >> 33) % span)


def test_math_int():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        rng = _Lcg(123456789)
        for i in range(200000):
            v = i - 100000
            acc += abs(v)
            acc += min(v, 0)
            acc += max(v, 0)
            acc += _clampi(v, -10, 10)
            acc += rng.rand(0, 100)
        checksum = acc
        times.append(now_ns() - t0)
    print("math_int = %d" % checksum, file=sys.stderr)
    record("math", "int", times)


def _range_float(n, lo, span):
    return [lo + float(i) / float(n) * span for i in range(n)]


# SIMD: the same element-wise math done as plain Python loops over lists,
# mirroring math.mfb's array (Float[]) overloads folded with collections::sum.
def test_math_simd():
    unit = _range_float(1024, -0.9, 1.8)     # [-0.9, 0.9)
    pos = _range_float(1024, 0.01, 4.0)      # (0, ~4]
    big = _range_float(1024, -1000.0, 2000.0)
    expo = _range_float(1024, -2.0, 4.0)
    lo = _range_float(1024, -5.0, 10.0)
    times = []
    checksum = 0.0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0.0
        for _rep in range(200):
            acc += sum(abs(x) for x in big)
            acc += float(sum(math.floor(x) for x in pos))
            acc += float(sum(math.ceil(x) for x in pos))
            acc += float(sum(_round_half_away(x) for x in pos))
            acc += sum(min(b, l) for b, l in zip(big, lo))
            acc += sum(max(b, l) for b, l in zip(big, lo))
            acc += sum(_clampf(b, -1.0, 1.0) for b in big)
            acc += sum(sqrt(x) for x in pos)
            acc += sum(log(x) for x in pos)
            acc += sum(log10(x) for x in pos)
            acc += sum(exp(x) for x in expo)
            acc += sum(sin(x) for x in unit)
            acc += sum(cos(x) for x in unit)
            acc += sum(tan(x) for x in unit)
            acc += sum(atan(x) for x in unit)
            acc += sum(asin(x) for x in unit)
            acc += sum(acos(x) for x in unit)
            acc += sum(mpow(p, e) for p, e in zip(pos, expo))
            acc += sum(atan2(u, p) for u, p in zip(unit, pos))
        checksum = acc
        times.append(now_ns() - t0)
    print("math_simd = %.3f" % checksum, file=sys.stderr)
    record("math", "simd", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_math_float()
    test_math_int()
    test_math_simd()
