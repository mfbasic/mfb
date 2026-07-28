"""Set-group benchmarks (Python `set` / collections:: over `Set OF T`, plan-63).

Mirrors benchmark/mfb/src/setops.mfb: `build` grows a set by repeated `add`
(half the inserts are duplicates, exercising the idempotent-hit path) and sums a
membership sweep; `ops` is one coverage row over the whole Set surface — union /
intersection / difference / symmetric_difference / issubset / issuperset /
isdisjoint / remove and a to-list/to-set round-trip. The checksums (20000 and
6006) match the mfb and C columns bit-for-bit.
"""
import sys

RUN = 1
now_ns = None
record = None


def test_set_build():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        s = set()
        for i in range(20000):
            s.add(i // 2)
        hits = 0
        for i in range(20000):
            if i in s:
                hits += 1
        checksum = len(s) + hits
        times.append(now_ns() - t0)
    print("set_build = %d" % checksum, file=sys.stderr)
    record("set", "build", times)


def test_set_ops():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        a = set()
        b = set()
        for i in range(1000):
            a.add(i)
            b.add(i + 500)
        u = a | b
        inter = a & b
        diff = a - b
        sym = a ^ b
        without_one = a - {0}
        from_list = set(list(u))
        flags = 0
        if inter <= a:               # isSubset(inter, a)
            flags += 1
        if u >= a:                   # isSuperset(u, a)
            flags += 2
        if diff.isdisjoint(b):       # isDisjoint(diff, b)
            flags += 4
        checksum = (len(u) + len(inter) + len(diff) + len(sym) +
                    len(without_one) + len(from_list) + flags)
        times.append(now_ns() - t0)
    print("set_ops = %d" % checksum, file=sys.stderr)
    record("set", "ops", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_set_build()
    test_set_ops()
