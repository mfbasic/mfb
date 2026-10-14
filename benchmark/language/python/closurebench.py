"""First-class function benchmarks (group `closure`).

Mirrors benchmark/language/mfb/src/closure.mfb (P1 coverage): a user function taking a
function parameter (called with a named def and with a lambda), and a function
returning a closure over a local, invoked many times, and a loop-body
accumulator updated through a for-each callback (a nested def with nonlocal).

Expected checksums: hof_param=2499997500000, closure_ret=34546050000,
capture_mut=1996501500.
"""
import sys

RUN = 1
now_ns = None
record = None


def clo_apply(f, x):
    return f(x)


def clo_step(x):
    return x * 2 + 1


def clo_make_affine(k, offset):
    scale = k * 2 + 1
    return lambda x: x * scale + offset


def test_closure_hof_param():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(1000000):
            acc += clo_apply(clo_step, i) + clo_apply(lambda x: x * 3 - 1, i)
        checksum = acc
        times.append(now_ns() - t0)
    print("test_closure_hof_param = %d" % checksum, file=sys.stderr)
    record("closure", "hof_param", times)


def test_closure_closure_ret():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for rep in range(100):
            f = clo_make_affine(rep % 7, rep)
            for i in range(10000):
                acc += f(i)
        checksum = acc
        times.append(now_ns() - t0)
    print("test_closure_closure_ret = %d" % checksum, file=sys.stderr)
    record("closure", "closure_ret", times)


def clo_for_each(xs, f):
    for x in xs:
        f(x)


def test_closure_capture_mut():
    xs = [(i * 7919) % 1000 for i in range(1000)]
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for rep in range(1000):
            scale = rep % 7 + 1
            total = 0

            def add(x):
                nonlocal total
                total += x * scale

            clo_for_each(xs, add)
            acc += total
        checksum = acc
        times.append(now_ns() - t0)
    print("test_closure_capture_mut = %d" % checksum, file=sys.stderr)
    record("closure", "capture_mut", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_closure_hof_param()
    test_closure_closure_ret()
    test_closure_capture_mut()
