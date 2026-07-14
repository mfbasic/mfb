"""Scalar / codepoint-processing benchmarks (group `scalarbench`).

Mirrors benchmark/mfb/src/scalarbench.mfb, which tracks plan-41's `Scalar`
primitive. Python peers use list(str)/ord/chr/str.isX in place of
toScalars/fromScalars/toInt/toScalar and the strings::isX predicates.

Classification and codepoint arithmetic run over ASCII so the counts and
transformed codepoints match mfb / C / Python. `roundtrip` counts code points of
a mixed-script string (matching mfb's scalar count).
"""
import sys

RUN = 1
now_ns = None
record = None


def test_scalar_roundtrip():
    # frag == mfb's "caf\u{00E9} \u{4E2D}\u{6587} rocket\u{1F680} nai\u{0308}ve
    # Stra\u{00DF}e " -- 30 code points (decomposed "naïve": i + U+0308).
    frag = "caf\u00e9 \u4e2d\u6587 rocket\U0001f680 nai\u0308ve Stra\u00dfe "
    base = frag * 8
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _i in range(5):
            scalars = list(base)
            acc += len(scalars)
            back = "".join(scalars)
            acc += len(back)
        checksum = acc
        times.append(now_ns() - t0)
    print("scalar_roundtrip = %d" % checksum, file=sys.stderr)
    record("scalarbench", "roundtrip", times)


def test_scalar_classify():
    base = "The Quick Brown Fox 123 JUMPS over 42 lazy Dogs! Now 7 Cats and 9 Owls."
    scalars = list(base)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        n_letter = n_digit = n_white = n_upper = n_lower = 0
        for _pass in range(2000):
            n_letter = n_digit = n_white = n_upper = n_lower = 0
            for sc in scalars:
                if sc.isalpha():
                    n_letter += 1
                if sc.isdigit():
                    n_digit += 1
                if sc.isspace():
                    n_white += 1
                if sc.isupper():
                    n_upper += 1
                if sc.islower():
                    n_lower += 1
        checksum = (n_letter + n_digit * 100 + n_white * 10000
                    + n_upper * 1000000 + n_lower * 100000000)
        times.append(now_ns() - t0)
    print("scalar_classify = %d" % checksum, file=sys.stderr)
    record("scalarbench", "classify", times)


def test_scalar_transform():
    base = "The Quick Brown Fox Jumps Over The Lazy Dog"
    scalars = list(base)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        total = 0
        for _pass in range(200):
            out = []
            for sc in scalars:
                cp = ord(sc)
                cp2 = cp
                if 97 <= cp <= 122:
                    cp2 = ((cp - 97 + 13) % 26) + 97
                if 65 <= cp <= 90:
                    cp2 = ((cp - 65 + 13) % 26) + 65
                out.append(chr(cp2))
            rebuilt = "".join(out)
            total = len(rebuilt)
            sumcp = 0
            for sc2 in out:
                sumcp += ord(sc2)
            total += sumcp
        checksum = total
        times.append(now_ns() - t0)
    print("scalar_transform = %d" % checksum, file=sys.stderr)
    record("scalarbench", "transform", times)


def test_scalar_listchurn():
    frag = "azbyAZ09 mkq!Wp"
    base = frag * 6
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        ascents = 0
        for _pass in range(2000):
            scalars = list(base)
            n = len(scalars)
            a = 0
            for i in range(n - 1):
                if ord(scalars[i]) < ord(scalars[i + 1]):
                    a += 1
            ascents = a
        checksum = ascents
        times.append(now_ns() - t0)
    print("scalar_listchurn = %d" % checksum, file=sys.stderr)
    record("scalarbench", "listchurn", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_scalar_roundtrip()
    test_scalar_classify()
    test_scalar_transform()
    test_scalar_listchurn()
