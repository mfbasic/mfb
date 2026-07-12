#!/usr/bin/env python3
"""Unified Python benchmark for the MFBASIC benchmark suite.

One function per micro-benchmark; each times its workload `run` times (default
10, override with `--run N`) with time.perf_counter_ns(), then records median /
average / min / max in milliseconds. Results are grouped and printed as:

    GROUP:
      NAME: MED, AVG, MIN, MAX

Every test prints its checksum to stderr so the implementations can be
cross-checked. The workloads mirror the mfb and C references so the columns line
up; the bignum test deliberately avoids Python's native pow() and does the same
base-2^28 limb-list arithmetic as the other two.
"""
import bitsbench
import csv
import io
import json
import list as listbench
import mapbench
import math
import mathbench
import os
import re
import stringbench
import sys
import tempfile
import threading
import time
import vectorbench
from math import (acos, asin, atan, atan2, cos, exp, log, log10, pow as mpow,
                  sin, sqrt, tan)

RUN = 10
RESULTS = []


def now_ns():
    return time.perf_counter_ns()


def record(group, name, times):
    s = sorted(times)
    n = len(s)
    if n % 2:
        med = float(s[n // 2])
    else:
        med = (s[n // 2 - 1] + s[n // 2]) / 2.0
    RESULTS.append({
        "group": group,
        "name": name,
        "med": med / 1e6,
        "avg": (sum(s) / n) / 1e6,
        "min": s[0] / 1e6,
        "max": s[-1] / 1e6,
    })


def print_results():
    print("# columns: median, average, min, max (milliseconds)")
    last = None
    for r in RESULTS:
        if r["group"] != last:
            print("\n%s:" % r["group"])
            last = r["group"]
        print("  %-12s: %10.3f, %10.3f, %10.3f, %10.3f"
              % (r["name"], r["med"], r["avg"], r["min"], r["max"]))


def tmp_path(name):
    return os.path.join(tempfile.gettempdir(), name)


# ===================================================================== #
# GROUP: recurse                                                        #
# ===================================================================== #

def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


def test_fib():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        checksum = fib(35)
        times.append(now_ns() - t0)
    print("fib = %d" % checksum, file=sys.stderr)
    record("recurse", "fib", times)


def ack(m, n):
    if m == 0:
        return n + 1
    if n == 0:
        return ack(m - 1, 1)
    return ack(m - 1, ack(m, n - 1))


def test_ackermann():
    times = []
    checksum = 0
    old = sys.getrecursionlimit()
    sys.setrecursionlimit(100000)
    for _ in range(RUN):
        t0 = now_ns()
        checksum = ack(3, 7)
        times.append(now_ns() - t0)
    sys.setrecursionlimit(old)
    print("ackermann = %d" % checksum, file=sys.stderr)
    record("recurse", "ackermann", times)


# ===================================================================== #
# GROUP: float                                                          #
# ===================================================================== #

def test_leibniz():
    times = []
    checksum = 0.0
    for _ in range(RUN):
        t0 = now_ns()
        s = 0.0
        sign = 1.0
        for k in range(1000000):
            s += sign / (2 * k + 1)
            sign = sign * -1.0
        checksum = 4.0 * s
        times.append(now_ns() - t0)
    print("leibniz = %.5f" % checksum, file=sys.stderr)
    record("float", "leibniz", times)


def test_nbody():
    PI = 3.141592653589793
    SOLAR_MASS = 4.0 * PI * PI
    DPY = 365.24
    times = []
    checksum = 0.0
    for _ in range(RUN):
        t0 = now_ns()
        x = [0.0, 4.84143144246472090, 8.34336671824457987, 1.28943695621391310e+01, 1.53796971148509165e+01]
        y = [0.0, -1.16032004402742839, 4.12479856412430479, -1.51111514016986312e+01, -2.59193146099879641e+01]
        z = [0.0, -1.03622044471123109e-01, -4.03523417114321381e-01, -2.23307578892655734e-01, 1.79258772950371181e-01]
        vx = [0.0, 1.66007664274403694e-03 * DPY, -2.76742510726862411e-03 * DPY, 2.96460137564761618e-03 * DPY, 2.68067772490389322e-03 * DPY]
        vy = [0.0, 7.69901118419740425e-03 * DPY, 4.99852801234917238e-03 * DPY, 2.37847173959480950e-03 * DPY, 1.62824170038242295e-03 * DPY]
        vz = [0.0, -6.90460016972063023e-05 * DPY, 2.30417297573763929e-05 * DPY, -2.96589568540237556e-05 * DPY, -9.51592254519715870e-05 * DPY]
        mass = [SOLAR_MASS, 9.54791938424326609e-04 * SOLAR_MASS, 2.85885980666130812e-04 * SOLAR_MASS,
                4.36624404335156298e-05 * SOLAR_MASS, 5.15138902046611451e-05 * SOLAR_MASS]

        px = py = pz = 0.0
        for i in range(5):
            px += vx[i] * mass[i]
            py += vy[i] * mass[i]
            pz += vz[i] * mass[i]
        vx[0] = -px / SOLAR_MASS
        vy[0] = -py / SOLAR_MASS
        vz[0] = -pz / SOLAR_MASS

        for _s in range(100000):
            for i in range(5):
                for j in range(i + 1, 5):
                    dx = x[i] - x[j]
                    dy = y[i] - y[j]
                    dz = z[i] - z[j]
                    d2 = dx * dx + dy * dy + dz * dz
                    mag = 0.01 / (d2 * math.sqrt(d2))
                    vx[i] -= dx * mass[j] * mag
                    vy[i] -= dy * mass[j] * mag
                    vz[i] -= dz * mass[j] * mag
                    vx[j] += dx * mass[i] * mag
                    vy[j] += dy * mass[i] * mag
                    vz[j] += dz * mass[i] * mag
            for i in range(5):
                x[i] += 0.01 * vx[i]
                y[i] += 0.01 * vy[i]
                z[i] += 0.01 * vz[i]

        e = 0.0
        for i in range(5):
            e += 0.5 * mass[i] * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i])
            for j in range(i + 1, 5):
                dx = x[i] - x[j]
                dy = y[i] - y[j]
                dz = z[i] - z[j]
                e -= mass[i] * mass[j] / math.sqrt(dx * dx + dy * dy + dz * dz)
        checksum = e
        times.append(now_ns() - t0)
    print("nbody = %.9f" % checksum, file=sys.stderr)
    record("float", "nbody", times)


def test_mandelbrot():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        w = h = 600
        maxiter = 100
        inset = 0
        for yy in range(h):
            im = -1.5 + 3.0 * (yy + 0.5) / h
            for xx in range(w):
                re_ = -2.0 + 3.0 * (xx + 0.5) / w
                zr = 0.0
                zi = 0.0
                escaped = False
                i = 0
                while i < maxiter:
                    nzr = zr * zr - zi * zi + re_
                    nzi = 2.0 * zr * zi + im
                    zr = nzr
                    zi = nzi
                    if zr * zr + zi * zi > 4.0:
                        escaped = True
                        i = maxiter
                    else:
                        i += 1
                if not escaped:
                    inset += 1
        checksum = inset
        times.append(now_ns() - t0)
    print("mandelbrot = %d" % checksum, file=sys.stderr)
    record("float", "mandelbrot", times)


# ===================================================================== #
# GROUP: math (each kernel run 2000 x 1000 times)                       #
# ===================================================================== #

def _math_kernel(label, fn, init, step):
    times = []
    checksum = 0.0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0.0
        for _rep in range(2000):
            v = init
            for _i in range(1000):
                acc += fn(v)
                v += step
        checksum = acc
        times.append(now_ns() - t0)
    print("%s = %.6f" % (label, checksum), file=sys.stderr)
    record("math", label, times)


def test_sin():
    _math_kernel("sin", sin, 0.001, 0.0015)


def test_cos():
    _math_kernel("cos", cos, 0.001, 0.0015)


def test_tan():
    _math_kernel("tan", tan, 0.001, 0.0015)


def test_atan2():
    _math_kernel("atan2", lambda v: atan2(v, 1.0 + v), 0.001, 0.0015)


def test_asin():
    _math_kernel("asin", asin, -0.999, 0.001998)


def test_acos():
    _math_kernel("acos", acos, -0.999, 0.001998)


def test_atan():
    _math_kernel("atan", atan, -0.999, 0.001998)


def test_exp():
    _math_kernel("exp", lambda v: exp(v * 0.1), 0.001, 0.005)


def test_log():
    _math_kernel("log", log, 0.001, 0.005)


def test_log10():
    _math_kernel("log10", log10, 0.001, 0.005)


def test_pow():
    _math_kernel("pow", lambda v: mpow(v, 1.5), 0.001, 0.005)


def test_sqrt():
    _math_kernel("sqrt", sqrt, 0.001, 0.005)


# ===================================================================== #
# GROUP: map / string / bits / vector coverage rows live in their own    #
# modules (mapbench.py, stringbench.py, bitsbench.py, vectorbench.py) and #
# the math float/int/simd rows in mathbench.py -- mirroring how the mfb   #
# side is split per package. main.py keeps the historical per-kernel math #
# rows above and the cross-cutting rows below.                           #
# ===================================================================== #

# ===================================================================== #
# GROUP: record                                                         #
# ===================================================================== #

def test_record_update():
    times = []
    checksum = 0
    for _ in range(RUN):
        recs = [{"n": i, "label": "p%d" % i} for i in range(100)]
        t0 = now_ns()
        for _pass in range(10):
            for j in range(100):
                recs[j] = {"n": recs[j]["n"] + 1, "label": recs[j]["label"]}
        times.append(now_ns() - t0)
        checksum = sum(r["n"] for r in recs)
    print("record_update = %d" % checksum, file=sys.stderr)
    record("record", "update", times)


# ===================================================================== #
# GROUP: bignum (base-2^28 limb lists; NOT native pow)                  #
# ===================================================================== #

MASK = 268435455


def bn_norm(a):
    n = len(a)
    while n > 1 and a[n - 1] == 0:
        n -= 1
    return a[:n]


def bn_cmp(a, b):
    la, lb = len(a), len(b)
    for i in range(max(la, lb) - 1, -1, -1):
        ai = a[i] if i < la else 0
        bi = b[i] if i < lb else 0
        if ai < bi:
            return -1
        if ai > bi:
            return 1
    return 0


def bn_add(a, b):
    la, lb = len(a), len(b)
    r = []
    c = 0
    for i in range(max(la, lb)):
        ai = a[i] if i < la else 0
        bi = b[i] if i < lb else 0
        s = ai + bi + c
        r.append(s & MASK)
        c = s >> 28
    if c:
        r.append(c)
    return r


def bn_sub(a, b):
    lb = len(b)
    r = []
    brw = 0
    for i in range(len(a)):
        bi = b[i] if i < lb else 0
        s = a[i] - bi - brw
        if s < 0:
            s += 268435456
            brw = 1
        else:
            brw = 0
        r.append(s)
    return bn_norm(r)


def bn_mul(a, b):
    la, lb = len(a), len(b)
    r = [0] * (la + lb)
    for i in range(la):
        c = 0
        ai = a[i]
        for j in range(lb):
            t = r[i + j] + ai * b[j] + c
            r[i + j] = t & MASK
            c = t >> 28
        r[i + lb] += c
    return bn_norm(r)


def bn_shl1(a):
    r = []
    c = 0
    for i in range(len(a)):
        t = (a[i] << 1) | c
        r.append(t & MASK)
        c = t >> 28
    if c:
        r.append(c)
    return r


def bn_mod(x, m):
    if bn_cmp(x, m) < 0:
        return x
    nbits = len(x) * 28
    r = [0]
    for i in range(nbits - 1, -1, -1):
        limb, off = divmod(i, 28)
        bit = (x[limb] >> off) & 1
        r = bn_shl1(r)
        if bit:
            r = bn_add(r, [1])
        if bn_cmp(r, m) >= 0:
            r = bn_sub(r, m)
    return r


def bn_modmul(a, b, m):
    return bn_mod(bn_mul(a, b), m)


P256 = [268435455, 268435455, 268435455, 4095, 0, 0, 16777216, 0, 268435455, 15]
GEN = [220077856, 27374017, 102176793, 20005201, 252711186, 12636384, 134810123, 5267568, 16909060]


def test_bignum_modmul():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        b = GEN
        for _i in range(200):
            b = bn_modmul(b, GEN, P256)
        checksum = sum(b)
        times.append(now_ns() - t0)
    print("bignum_modmul = %d" % checksum, file=sys.stderr)
    record("bignum", "modmul", times)


def test_bignum_modexp():
    e = 6822318947648322238
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        r = [1]
        b = GEN
        for i in range(63):
            if (e >> i) & 1:
                r = bn_modmul(r, b, P256)
            b = bn_modmul(b, b, P256)
        checksum = sum(r)
        times.append(now_ns() - t0)
    print("bignum_modexp = %d" % checksum, file=sys.stderr)
    record("bignum", "modexp", times)


# ===================================================================== #
# GROUP: parse (input generated to a temp file first, then timed)       #
# ===================================================================== #

def test_parse_csv():
    path = tmp_path("py-bench-parse-csv.csv")
    with open(path, "w") as f:
        for i in range(2000):
            f.write("%d,%d,%d\n" % (i, i + 1, i + 2))
    times = []
    checksum = 0
    for _ in range(RUN):
        with open(path) as f:
            text = f.read()
        t0 = now_ns()
        grid = list(csv.reader(io.StringIO(text)))
        total = 0
        for row in grid:
            for cell in row:
                total += int(cell)
        times.append(now_ns() - t0)
        checksum = total
    os.remove(path)
    print("parse_csv = %d" % checksum, file=sys.stderr)
    record("parse", "csv", times)


def test_parse_json():
    path = tmp_path("py-bench-parse-json.json")
    nums = ",".join(str(i) for i in range(5000))
    with open(path, "w") as f:
        f.write('{"nums":[' + nums + '],"tail":5000}')
    times = []
    checksum = ""
    for _ in range(RUN):
        with open(path) as f:
            text = f.read()
        t0 = now_ns()
        value = json.loads(text)
        tail = json.dumps(value["tail"])
        times.append(now_ns() - t0)
        checksum = tail
    os.remove(path)
    print("parse_json = %s" % checksum, file=sys.stderr)
    record("parse", "json", times)


def test_parse_regex():
    path = tmp_path("py-bench-parse-regex.txt")
    with open(path, "w") as f:
        f.write(" ".join(str(i) for i in range(200)))
    pattern = re.compile("[0-9]+")
    times = []
    checksum = 0
    for _ in range(RUN):
        with open(path) as f:
            text = f.read()
        t0 = now_ns()
        matches = pattern.findall(text)
        times.append(now_ns() - t0)
        checksum = len(matches)
    os.remove(path)
    print("parse_regex = %d" % checksum, file=sys.stderr)
    record("parse", "regex", times)


# ===================================================================== #
# GROUP: io (file-based; matches the mfb line-by-line workload)         #
# ===================================================================== #

def write_lines(path, count):
    with open(path, "w") as f:
        for i in range(count):
            f.write("%d\n" % i)
    return count


def test_io_write():
    path = tmp_path("py-bench-io-write.txt")
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        checksum = write_lines(path, 20000)
        times.append(now_ns() - t0)
    os.remove(path)
    print("io_write = %d" % checksum, file=sys.stderr)
    record("io", "write", times)


def test_io_read():
    path = tmp_path("py-bench-io-read.txt")
    write_lines(path, 20000)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        lines = 0
        with open(path) as f:
            for _line in f:
                lines += 1
        checksum = lines
        times.append(now_ns() - t0)
    os.remove(path)
    print("io_read = %d" % checksum, file=sys.stderr)
    record("io", "read", times)


# ===================================================================== #
# GROUP: primes                                                         #
# ===================================================================== #

def is_prime(n):
    if n < 2:
        return False
    i = 2
    while i * i <= n:
        if n % i == 0:
            return False
        i += 1
    return True


def test_primes():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        primes = []
        candidate = 2
        while len(primes) < 1000:
            if is_prime(candidate):
                primes.append(candidate)
            candidate += 1
        checksum = primes[-1]
        times.append(now_ns() - t0)
    print("primes = %d" % checksum, file=sys.stderr)
    record("primes", "primes", times)


# ===================================================================== #
# GROUP: thread (4 workers x 10,000,000; GIL serializes them)           #
# ===================================================================== #

def _sum_chunk(start, out, idx):
    total = 0
    for i in range(start, start + 10000000):
        total += i
    out[idx] = total


def test_thread_sum():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        out = [0, 0, 0, 0]
        workers = [threading.Thread(target=_sum_chunk, args=(k * 10000000, out, k)) for k in range(4)]
        for w in workers:
            w.start()
        for w in workers:
            w.join()
        checksum = sum(out)
        times.append(now_ns() - t0)
    print("thread_sum = %d" % checksum, file=sys.stderr)
    record("thread", "sum", times)


# ===================================================================== #

def main():
    global RUN
    args = sys.argv[1:]
    for i, a in enumerate(args):
        if a == "--run" and i + 1 < len(args):
            try:
                v = int(args[i + 1])
                if v >= 1:
                    RUN = v
            except ValueError:
                pass
    print("running each test %d time(s)" % RUN, file=sys.stderr)

    test_fib()
    test_ackermann()

    test_leibniz()
    test_nbody()
    test_mandelbrot()

    test_sin(); test_cos(); test_tan(); test_atan2()
    test_asin(); test_acos(); test_atan()
    test_exp(); test_log(); test_log10(); test_pow(); test_sqrt()

    # math coverage rows (float/int/simd)
    mathbench.run_all(RUN, now_ns, record)

    # list group + liststr rows
    listbench.run_all(RUN, now_ns, record)

    # map group (set/lookup/int_ops/str_ops)
    mapbench.run_all(RUN, now_ns, record)

    # string group (concat/case/search/slice/unicode)
    stringbench.run_all(RUN, now_ns, record)

    # bits group (ops)
    bitsbench.run_all(RUN, now_ns, record)

    test_record_update()

    test_bignum_modmul()
    test_bignum_modexp()

    test_parse_csv()
    test_parse_json()
    test_parse_regex()

    test_io_write()
    test_io_read()

    # vector group (math/float/int)
    vectorbench.run_all(RUN, now_ns, record)

    test_primes()

    test_thread_sum()

    print_results()


if __name__ == "__main__":
    main()
