"""Float / transcendental pipeline benchmarks.

Mirrors benchmark/mfb/src/mathpipe.mfb: matmul (group `float`, dense FMA) plus
the `mathpipe` group (dft, stats). `finance` is mfb-only (Money has no Python
peer) and is deliberately NOT mirrored here.

matmul / stats are pure IEEE arithmetic in a fixed loop order, so their %.6f
checksums are bit-identical to the mfb reference. dft's checksum is the two
dominant bin indices, robust to low-bit sin/cos differences.
"""
import math
import sys

RUN = 1
now_ns = None
record = None


def test_matmul():
    n = 64
    a = []
    b = []
    for i in range(n):
        for j in range(n):
            a.append(float((i * 7 + j * 3) % 97) / 97.0)
            b.append(float((i * 5 + j * 13) % 89) / 89.0)
    times = []
    checksum = 0.0
    for _ in range(RUN):
        t0 = now_ns()
        s = 0.0
        for i in range(n):
            for j in range(n):
                acc = 0.0
                for k in range(n):
                    acc += a[i * n + k] * b[k * n + j]
                s += acc
        checksum = s
        times.append(now_ns() - t0)
    print("matmul = %.6f" % checksum, file=sys.stderr)
    record("float", "matmul", times)


def test_dft():
    n = 256
    pi = 3.141592653589793
    two_pi = 2.0 * pi
    sig = []
    for t in range(n):
        tf = float(t)
        sig.append(math.cos(two_pi * 5.0 * tf / float(n)) + 0.5 * math.cos(two_pi * 20.0 * tf / float(n)))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        best1 = 0
        best2 = 0
        mag1 = -1.0
        mag2 = -1.0
        for k in range(n // 2):
            re = 0.0
            im = 0.0
            kf = float(k)
            for t in range(n):
                ang = two_pi * kf * float(t) / float(n)
                sv = sig[t]
                re += sv * math.cos(ang)
                im -= sv * math.sin(ang)
            mag = re * re + im * im
            if mag > mag1:
                mag2 = mag1
                best2 = best1
                mag1 = mag
                best1 = k
            elif mag > mag2:
                mag2 = mag
                best2 = k
        checksum = best1 * 1000 + best2
        times.append(now_ns() - t0)
    print("dft = %d" % checksum, file=sys.stderr)
    record("mathpipe", "dft", times)


def test_stats():
    n = 200000
    xs = []
    for i in range(n):
        xs.append(float(i % 1000))
    times = []
    checksum = ""
    for _ in range(RUN):
        t0 = now_ns()
        s = 0.0
        for v in xs:
            s += v
        mean = s / float(n)
        sq = 0.0
        for v in xs:
            d = v - mean
            sq += d * d
        variance = sq / float(n)
        _stddev = math.sqrt(variance)
        checksum = "%.6f,%.6f" % (mean, variance)
        times.append(now_ns() - t0)
    print("stats = %s" % checksum, file=sys.stderr)
    record("mathpipe", "stats", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_matmul()
    test_dft()
    test_stats()
