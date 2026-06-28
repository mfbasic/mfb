#!/usr/bin/env python3
"""gen_inputs.py — representative + boundary input sets for the math-kernel
reference capture (plan-01-simd Phase 5, Validation Plan).

Emits, for one transcendental, the stream of input bit-patterns that capture_ref
feeds through macOS libm. Selection is deterministic so re-running on the
reference Mac reproduces the same vectors bit-for-bit. Inputs are chosen to
exercise everything a <=1 ULP claim must cover:

  * the whole supported domain (a dense uniform sweep),
  * domain boundaries and special values (0, +-1, +-pi multiples, subnormals,
    powers of two, values straddling each kernel's range-reduction breakpoints),
  * range-reduction stress for trig (large multiples of pi/2, up to ~2**50 where
    a double still resolves individual radians),
  * near-1 walks for log/log10 (where relative error is most fragile),
  * a fixed pseudo-random fill (seeded, so it is reproducible) for coverage
    between the structured points.

Only IN-DOMAIN inputs are emitted: out-of-domain handling (asin/acos outside
[-1,1], log of <=0, pow of a negative base with a *fractional* exponent, ...) is
the kernels' per-lane error path and is covered by the _invalid/_rt function
tests, not by ULP vectors. Note pow of a *negative base with an integer
exponent* IS in-domain — the kernel matches libm there ((-2)^3 = -8;
plan-01-libm-kernels §4.4) — so those vectors are emitted.

Output: lines of lowercase 16-hex-digit IEEE-754 bit patterns (big-endian byte
order == the integer bit pattern == capture_ref's "%016llx"). Unary functions
emit one value per line; atan2/pow/fmod emit two (x then y).

Usage:  python3 gen_inputs.py <fn>        # fn = exp|log|...|atan2|pow|fmod
        python3 gen_inputs.py --list      # list known functions
"""
import math
import struct
import sys

# Deterministic PRNG (no global random state, no external deps) so the captured
# vectors are reproducible on any reference Mac. Splitmix64 -> double in a range.
class Rng:
    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFFFFFFFFFF

    def next_u64(self):
        self.s = (self.s + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
        z = self.s
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
        return z ^ (z >> 31)

    def uniform(self, lo, hi):
        frac = self.next_u64() / 2.0**64
        return lo + (hi - lo) * frac


def bits(x):
    """16-hex-digit IEEE-754 bit pattern of a Python float (== C %016llx)."""
    return struct.pack(">d", float(x)).hex()


def linspace(lo, hi, n):
    if n == 1:
        return [lo]
    step = (hi - lo) / (n - 1)
    return [lo + step * i for i in range(n)]


def around(x, n=4):
    """x and its n nearest doubles on each side — straddles breakpoints exactly."""
    out = [x]
    up = x
    dn = x
    for _ in range(n):
        up = math.nextafter(up, math.inf)
        dn = math.nextafter(dn, -math.inf)
        out.append(up)
        out.append(dn)
    return out


PI = math.pi
LN2 = math.log(2.0)

# Per-function input plans. Each returns a list of floats (unary) or (x, y)
# tuples (binary). Kept in one place so domains and reduction breakpoints are
# auditable alongside the kernel design in plan §4.6.

def inputs_exp():
    pts = []
    # exp overflows near 709.78; underflows to 0 near -745. Stay just inside the
    # finite range so libm returns comparable normals/subnormals.
    pts += linspace(-745.0, 709.0, 600)
    pts += around(0.0)
    for k in range(-12, 13):                 # straddle n*ln2 reduction points
        pts += around(k * LN2)
    pts += [-1.0, 1.0, 0.5, -0.5, LN2, -LN2, 88.0, -88.0, 700.0, -700.0]
    rng = Rng(0xE0)
    pts += [rng.uniform(-744.0, 709.0) for _ in range(400)]
    return pts


def inputs_log_like():
    # log / log10: domain (0, +inf). Densest near 1, plus a wide exponent sweep.
    pts = []
    pts += around(1.0, 8)                     # most fragile relative-error region
    pts += linspace(0.5, 2.0, 200)            # one binade around the mantissa fold
    # span the whole exponent range geometrically
    e = -300
    while e <= 300:
        pts.append(2.0 ** e)
        pts.append(10.0 ** (e / 3.0) if e != 0 else 1.0)
        e += 1
    pts += [math.sqrt(0.5), math.sqrt(2.0)]   # mantissa-fold breakpoints
    pts += [1e-308, 5e-324, 1e308, math.e, 10.0]
    rng = Rng(0x106)
    pts += [math.exp(rng.uniform(-700.0, 700.0)) for _ in range(400)]
    return [p for p in pts if p > 0.0 and math.isfinite(p)]


def inputs_trig():
    # sin/cos/tan: full real line in principle; test up to where a double still
    # resolves single radians so range reduction is meaningfully checked.
    pts = []
    pts += linspace(-2 * PI, 2 * PI, 400)
    for k in range(-8, 9):                     # straddle k*pi/2 quadrant edges
        pts += around(k * PI / 2.0)
    pts += around(0.0)
    # range-reduction stress: large arguments, exact and offset
    for mag in (1e3, 1e6, 1e9, 1e12, 1e15, 2.0**40, 2.0**50):
        pts += [mag, -mag, mag + 0.5, mag + PI / 4.0]
    rng = Rng(0x21)
    pts += [rng.uniform(-1e6, 1e6) for _ in range(300)]
    pts += [rng.uniform(-PI, PI) for _ in range(200)]
    return pts


def inputs_tan():
    # tan = sin/cos; avoid the exact poles (cos == 0) where libm returns a huge
    # but finite value that is meaningless to compare at <=1 ULP.
    return [p for p in inputs_trig()
            if abs(math.cos(p)) > 1e-9]


def inputs_asin_acos():
    pts = []
    pts += linspace(-1.0, 1.0, 400)
    pts += around(0.0)
    pts += around(1.0, 6)
    pts += around(-1.0, 6)
    pts += around(0.5)
    pts += around(-0.5)
    pts += [math.sqrt(0.5), -math.sqrt(0.5)]   # asin/acos reduction breakpoint
    rng = Rng(0xA51)
    pts += [rng.uniform(-1.0, 1.0) for _ in range(400)]
    return [p for p in pts if -1.0 <= p <= 1.0]


def inputs_atan():
    pts = []
    pts += linspace(-4.0, 4.0, 300)
    pts += around(0.0)
    pts += around(1.0)
    pts += around(-1.0)
    # 2 - sqrt(3) is the classic atan argument-reduction breakpoint
    bp = 2.0 - math.sqrt(3.0)
    pts += around(bp)
    pts += around(-bp)
    pts += [1e-20, -1e-20, 1e6, -1e6, 1e15, -1e15, 1e300, -1e300]
    rng = Rng(0xA7A)
    pts += [math.tan(rng.uniform(-PI / 2 * 0.999, PI / 2 * 0.999)) for _ in range(300)]
    return [p for p in pts if math.isfinite(p)]


def inputs_atan2():
    rng = Rng(0xA72)
    pts = []
    sample = [-1e3, -10.0, -1.0, -0.5, 0.0, 0.5, 1.0, 10.0, 1e3]
    for y in sample:
        for x in sample:
            if x == 0.0 and y == 0.0:
                continue
            pts.append((y, x))
    for _ in range(400):
        ang = rng.uniform(-PI, PI)
        r = math.exp(rng.uniform(-20.0, 20.0))
        pts.append((r * math.sin(ang), r * math.cos(ang)))
    return pts


def inputs_pow():
    rng = Rng(0x909)
    pts = []
    bases = [0.5, 1.0, 1.5, 2.0, math.e, 10.0, 0.1, 100.0]
    exps = [-3.0, -2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0, 3.0, 0.25]
    for b in bases:
        for e in exps:
            pts.append((b, e))
    pts += [(2.0, 10.0), (2.0, -10.0), (10.0, 300.0), (10.0, -300.0)]
    for _ in range(400):
        b = math.exp(rng.uniform(-5.0, 5.0))       # positive base (in-domain)
        e = rng.uniform(-20.0, 20.0)
        v = math.pow(b, e)
        if math.isfinite(v) and v > 0.0:
            pts.append((b, e))
    # Negative base with an INTEGER exponent — in-domain since the kernel matches
    # libm's sign/integer handling (plan-01-libm-kernels §4.4). Both parities so
    # the odd/even sign branch is exercised; non-integer exponents stay out of
    # domain (the per-lane error path) and are deliberately not emitted here.
    neg_bases = [-0.5, -1.0, -1.5, -2.0, -math.e, -10.0, -0.1, -100.0]
    int_exps = [-4.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 10.0, 11.0]
    for b in neg_bases:
        for e in int_exps:
            v = math.pow(b, e)
            if math.isfinite(v):
                pts.append((b, e))
    for _ in range(200):
        b = -math.exp(rng.uniform(-5.0, 5.0))      # negative base
        e = float(int(rng.uniform(-12.0, 12.0)))   # integer exponent
        v = math.pow(b, e)
        if math.isfinite(v):
            pts.append((b, e))
    return pts


def inputs_fmod():
    # fmod(a, b) = a - n*b is EXACT, so the kernel must be bit-identical (0 ULP)
    # to libm. Coverage targets the bitwise remainder algorithm: a wide spread of
    # exponent gaps (drives the subtractive-reduction iteration count), every
    # sign combination (result takes a's sign), and the boundary cases libm
    # special-cases — |a|<|b| (returns a), |a|==|b| and exact multiples (returns
    # +-0 with a's sign). A zero divisor never reaches the kernel (the Float MOD
    # ErrFloatDomain pre-check guards it), so b == 0 is not emitted.
    rng = Rng(0xF0D)
    pts = []
    structured = [
        (7.5, 2.0), (7.5, -2.0), (-7.5, 2.0), (-7.5, -2.0),  # sign matrix
        (1.0, 3.0), (-1.0, 3.0),                              # |a| < |b| -> a
        (5.0, 5.0), (-5.0, 5.0), (6.0, 2.0), (-9.0, 3.0),     # exact multiple -> +-0
        (5.3, 5.3), (-5.3, 5.3),                              # |a| == |b| -> +-0
        (1e300, 1e-300), (1e300, 3.0), (1e-300, 1e-308),      # huge exponent gaps
        (123456.789, 1.0), (123456.789, 0.5),                 # near-integer reductions
        (3.0, 1e300), (2.5, 1e16),                            # |a| << |b| -> a
        (1.0, math.ldexp(1.0, -1070)),                        # subnormal divisor
        (math.ldexp(1.0, -1070), math.ldexp(1.0, -1074)),     # subnormal a and b
        (math.pi, math.e), (-math.pi, math.e), (math.e, math.pi),
    ]
    pts += structured
    # Powers-of-two operands: the remainder is exact and stresses the exponent
    # alignment with no mantissa noise.
    p2 = [math.ldexp(1.0, k) for k in (-40, -10, -1, 0, 1, 10, 40)]
    for a in p2:
        for b in p2:
            pts.append((a, b))
            pts.append((-a, b))
    # Pseudo-random fill across many magnitudes and both signs.
    for _ in range(500):
        a = math.copysign(math.exp(rng.uniform(-40.0, 40.0)), rng.uniform(-1.0, 1.0))
        b = math.copysign(math.exp(rng.uniform(-40.0, 40.0)), rng.uniform(-1.0, 1.0))
        if b == 0.0:
            continue
        pts.append((a, b))
    return pts


PLANS = {
    "exp": inputs_exp,
    "log": inputs_log_like,
    "log10": inputs_log_like,
    "sin": inputs_trig,
    "cos": inputs_trig,
    "tan": inputs_tan,
    "asin": inputs_asin_acos,
    "acos": inputs_asin_acos,
    "atan": inputs_atan,
    "atan2": inputs_atan2,
    "pow": inputs_pow,
    "fmod": inputs_fmod,
}
BINARY = {"atan2", "pow", "fmod"}


def dedup(seq):
    """Stable dedup by exact bit pattern (preserves first occurrence/order)."""
    seen = set()
    out = []
    for v in seq:
        key = bits(v) if not isinstance(v, tuple) else (bits(v[0]), bits(v[1]))
        if key in seen:
            continue
        seen.add(key)
        out.append(v)
    return out


def main(argv):
    if len(argv) != 2 or argv[1] in ("-h", "--help"):
        sys.stderr.write(__doc__)
        return 2
    if argv[1] == "--list":
        print(" ".join(sorted(PLANS)))
        return 0
    fn = argv[1]
    if fn not in PLANS:
        sys.stderr.write(f"gen_inputs.py: unknown function '{fn}'\n")
        return 2

    pts = dedup(PLANS[fn]())
    out = []
    if fn in BINARY:
        for x, y in pts:
            out.append(f"{bits(x)} {bits(y)}")
    else:
        for x in pts:
            out.append(bits(x))
    sys.stdout.write("\n".join(out) + "\n")
    sys.stderr.write(f"{fn}: {len(out)} inputs\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
