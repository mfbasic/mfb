"""Arbitrary-precision integer benchmarks (group `bigint`).

Mirrors benchmark/mfb/src/bigint.mfb: the `big::` package against CPython's
native arbitrary-precision `int` (its standard means of doing the job). C has no
bignum in libc and the suite vendors none, so the C column prints `--`.

Rows (checksums must match mfb bit-for-bit):
  pidigits   Benchmarks Game spigot, first PI_DIGITS digits of pi; checksum folds
             the digits as fold = (fold*10 + d) mod 1000000007
  factorial  FACT_N! rendered to decimal; checksum = decimal digit sum + length
  modpow     RSA-2048 public-key shape: MODPOW_BASES bases ^ 65537 mod a fixed
             2048-bit odd modulus; checksum = sum of each result's low 32 bits
  gcd        GCD_PAIRS Fibonacci pairs (Euclid's worst case) parsed from decimal
             text, gcd'd, and rendered back; checksum = sum(len(text) + g mod 1e9+7)
"""
import math
import sys

RUN = 1
now_ns = None
record = None

PI_DIGITS = 400
FACT_N = 2000
MODPOW_BASES = 16
GCD_PAIRS = 200
FOLD_MOD = 1000000007


def pidigits(n):
    acc, den, num = 0, 1, 1
    k = 0
    i = 0
    fold = 0
    while i < n:
        k += 1
        k2 = k * 2 + 1
        acc = (acc + num + num) * k2
        den = den * k2
        num = num * k
        if num > acc:
            continue
        d = (num * 3 + acc) // den
        if d != (num * 4 + acc) // den:
            continue
        fold = (fold * 10 + d) % FOLD_MOD
        i += 1
        acc = (acc - den * d) * 10
        num = num * 10
    return fold


def test_bigint_pidigits():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        checksum = pidigits(PI_DIGITS)
        times.append(now_ns() - t0)
    print("test_bigint_pidigits = %d" % checksum, file=sys.stderr)
    record("bigint", "pidigits", times)


def test_bigint_factorial():
    # FACT_N! has 5736 digits; CPython >= 3.11 caps int<->str conversion at 4300
    # digits by default (a DoS guard), so lift the cap for this row.
    if hasattr(sys, "set_int_max_str_digits"):
        sys.set_int_max_str_digits(0)
    times = []
    text = ""
    for _ in range(RUN):
        t0 = now_ns()
        text = str(math.factorial(FACT_N))
        times.append(now_ns() - t0)
    checksum = sum(ord(c) - 48 for c in text) + len(text)
    print("test_bigint_factorial = %d" % checksum, file=sys.stderr)
    record("bigint", "factorial", times)


def test_bigint_modpow():
    modulus = (1 << 2048) - 1557
    e = 65537
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for b in range(2, 2 + MODPOW_BASES):
            r = pow(b, e, modulus)
            acc += r % 4294967296
        checksum = acc
        times.append(now_ns() - t0)
    print("test_bigint_modpow = %d" % checksum, file=sys.stderr)
    record("bigint", "modpow", times)


def gcd_texts():
    # Untimed setup: F(0..) as decimal text; pair i is (F(500+i), F(700+2i)).
    fib = [0, 1]
    while len(fib) < 700 + 2 * GCD_PAIRS:
        fib.append(fib[-1] + fib[-2])
    a = [str(fib[500 + i]) for i in range(GCD_PAIRS)]
    b = [str(fib[700 + 2 * i]) for i in range(GCD_PAIRS)]
    return a, b


def test_bigint_gcd():
    ta, tb = gcd_texts()
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(GCD_PAIRS):
            g = math.gcd(int(ta[i]), int(tb[i]))
            s = str(g)
            acc += len(s) + g % FOLD_MOD
        checksum = acc
        times.append(now_ns() - t0)
    print("test_bigint_gcd = %d" % checksum, file=sys.stderr)
    record("bigint", "gcd", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_bigint_pidigits()
    test_bigint_factorial()
    test_bigint_modpow()
    test_bigint_gcd()
