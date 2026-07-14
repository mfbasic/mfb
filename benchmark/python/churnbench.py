"""Churn-group benchmarks (list + map grow / shift / materialize hot paths).

Mirrors benchmark/mfb/src/listchurn.mfb (group `listchurn`) and
benchmark/mfb/src/mapchurn.mfb (group `mapchurn`). These grow a real collection
in a loop, unlike the per-member `list`/`map` rows that touch each op once over a
pre-built collection. Native Python list/dict do comparable materialized work so
the checksums line up with the mfb reference.
"""
import sys

RUN = 1
now_ns = None
record = None


# --------------------------------------------------------------------------- #
# GROUP: listchurn                                                             #
# --------------------------------------------------------------------------- #

def test_listchurn_append():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        nums = []
        for i in range(20000):
            nums.append(i)
        sumv = 0
        for v in nums:
            sumv += v
        checksum = sumv
        times.append(now_ns() - t0)
    print("listchurn_append = %d" % checksum, file=sys.stderr)
    record("listchurn", "append", times)


def test_listchurn_prepend():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        nums = []
        for i in range(2000):
            nums.insert(0, i)
        sumv = 0
        for v in nums:
            sumv += v
        checksum = sumv
        times.append(now_ns() - t0)
    print("listchurn_prepend = %d" % checksum, file=sys.stderr)
    record("listchurn", "prepend", times)


def test_listchurn_nested():
    outer = 200
    inner = 20
    passes = 20
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _p in range(passes):
            nested = []
            for i in range(outer):
                row = []
                for j in range(inner):
                    row.append(i * inner + j)
                nested.append(row)
            flat = [v for row in nested for v in row]
            acc += len(flat)
            sf = 0
            for v in flat:
                sf += v
            acc += sf
            groups = {}
            for v in flat:
                groups.setdefault(v % 100, []).append(v)
            acc += len(groups)
        checksum = acc
        times.append(now_ns() - t0)
    print("listchurn_nested = %d" % checksum, file=sys.stderr)
    record("listchurn", "nested", times)


# --------------------------------------------------------------------------- #
# GROUP: mapchurn                                                              #
# --------------------------------------------------------------------------- #

def test_mapchurn_grow():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        m = {}
        for i in range(5000):
            m[str(i)] = i
        sumv = 0
        for i in range(5000):
            if str(i) in m:
                sumv += m[str(i)]
        checksum = sumv
        times.append(now_ns() - t0)
    print("mapchurn_grow = %d" % checksum, file=sys.stderr)
    record("mapchurn", "grow", times)


def test_mapchurn_churn():
    base = 500
    cycles = 4000
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        m = {}
        for i in range(base):
            m[str(i)] = i
        removed = 0
        for stp in range(cycles):
            newkey = base + stp
            m[str(newkey)] = newkey
            if str(stp) in m:
                del m[str(stp)]
                removed += 1
        sumv = 0
        for v in m.values():
            sumv += v
        checksum = sumv + removed
        times.append(now_ns() - t0)
    print("mapchurn_churn = %d" % checksum, file=sys.stderr)
    record("mapchurn", "churn", times)


def test_mapchurn_iterate():
    n = 1000
    passes = 100
    times = []
    checksum = 0
    for _ in range(RUN):
        m = {}
        for i in range(n):
            m[str(i)] = i
        other = {}
        for i in range(n, n + 10):
            other[str(i)] = i
        t0 = now_ns()
        acc = 0
        for _p in range(passes):
            ks = list(m.keys())
            acc += len(ks)
            sv = 0
            for v in m.values():
                sv += v
            acc += sv
            dbl = {k: v + v for k, v in m.items()}
            acc += dbl["10"]
            mg = dict(m)
            mg.update(other)
            acc += len(list(mg.keys()))
        checksum = acc
        times.append(now_ns() - t0)
    print("mapchurn_iterate = %d" % checksum, file=sys.stderr)
    record("mapchurn", "iterate", times)


def run_listchurn(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_listchurn_append()
    test_listchurn_prepend()
    test_listchurn_nested()


def run_mapchurn(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_mapchurn_grow()
    test_mapchurn_churn()
    test_mapchurn_iterate()
