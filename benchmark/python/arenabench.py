"""Arena-stress benchmarks (group `arena`).

Mirrors benchmark/mfb/src/arena.mfb: mixed-size transient churn, long+short-lived
mix, and grow/shrink. These are the mfb runtime's plan-39-A regression gate; the
Python allocator is unaffected, so these mirrors keep the same tiny counts only
so the table lines up. Native Python list/str reproduce the exact arithmetic, so
the checksums match the mfb reference.
"""
import sys

RUN = 1
now_ns = None
record = None


def test_arena_transient():
    base = "abcdefghij"
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(400):
            size = (i % 16) + 1
            tmp = []
            for k in range(size):
                tmp.append(k)
            acc += sum(tmp)
            slicelen = (i % 7) + 1
            s = base[:slicelen]
            acc += len(s)
        checksum = acc
        times.append(now_ns() - t0)
    print("arena_transient = %d" % checksum, file=sys.stderr)
    record("arena", "transient", times)


def test_arena_mixed():
    long_lived = []
    for i in range(1000):
        long_lived.append(i)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(400):
            size = (i % 20) + 1
            tmp = []
            for k in range(size):
                tmp.append(k)
            acc += sum(tmp)
            s = "ab" * ((i % 5) + 1)
            acc += len(s)
        acc += len(long_lived)
        checksum = acc
        times.append(now_ns() - t0)
    print("arena_mixed = %d" % checksum, file=sys.stderr)
    record("arena", "mixed", times)


def test_arena_growshrink():
    grow = 100
    cycles = 200
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _cycle in range(cycles):
            xs = []
            for k in range(grow):
                xs.append(k)
            head = xs[:10]
            tail = xs[grow - 10:]
            acc += len(head) + len(tail)
        checksum = acc
        times.append(now_ns() - t0)
    print("arena_growshrink = %d" % checksum, file=sys.stderr)
    record("arena", "growshrink", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_arena_transient()
    test_arena_mixed()
    test_arena_growshrink()
