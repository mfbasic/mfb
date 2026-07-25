"""Map-group benchmarks (dict / collections:: over maps).

Mirrors benchmark/mfb/src/map.mfb: the two historical timing rows (set, lookup)
moved out of main.py, plus consolidated coverage rows (int_ops, str_ops) that
exercise every Map-shaped collections:: member -- merge, mapValues, keys,
values, hasKey, get, getOr, removeKey.
"""
import sys

RUN = 1
now_ns = None
record = None


def test_map_set():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        m = {}
        for i in range(1000):
            m[str(i)] = i
        total = 0
        for i in range(1000):
            total += m[str(i)]
        checksum = total
        times.append(now_ns() - t0)
    print("map_set = %d" % checksum, file=sys.stderr)
    record("map", "set", times)


def test_map_lookup():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        m = {}
        for i in range(20000):
            m[i] = i
        total = 0
        for i in range(20000):
            total += m[i]
        checksum = total
        times.append(now_ns() - t0)
    print("map_lookup = %d" % checksum, file=sys.stderr)
    record("map", "lookup", times)


# Integer values, String keys: every Map-shaped collections:: member.
def test_map_int_ops():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _pass in range(50):
            m = {}
            for i in range(200):
                m[str(i)] = i
            other = {}
            for i in range(200, 250):
                other[str(i)] = i
            # merge preferB=TRUE: keys in `other` win on collision.
            merged = dict(m)
            merged.update(other)
            # mapValues: double every int.
            doubled = {k: v + v for k, v in merged.items()}
            ks = list(doubled.keys())
            vs = list(doubled.values())
            acc += len(ks) + len(vs)
            if "10" in doubled:
                acc += doubled["10"]
            acc += doubled.get("missing", -1)
            pruned = dict(doubled)
            pruned.pop("0", None)
            acc += len(list(pruned.keys()))
        checksum = acc
        times.append(now_ns() - t0)
    print("map_int_ops = %d" % checksum, file=sys.stderr)
    record("map", "int_ops", times)


# String values, String keys: the same members over String payloads.
def test_map_str_ops():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _pass in range(50):
            m = {}
            for i in range(200):
                m[str(i)] = "v" + str(i)
            other = {}
            for i in range(200, 250):
                other[str(i)] = "v" + str(i)
            merged = dict(m)
            merged.update(other)
            # mapValues: append "!" to every string.
            tagged = {k: v + "!" for k, v in merged.items()}
            ks = list(tagged.keys())
            vs = list(tagged.values())
            acc += len(ks) + len(vs)
            if "10" in tagged:
                acc += len(tagged["10"])
            acc += len(tagged.get("missing", "none"))
            pruned = dict(tagged)
            pruned.pop("0", None)
            acc += len(list(pruned.keys()))
        checksum = acc
        times.append(now_ns() - t0)
    print("map_str_ops = %d" % checksum, file=sys.stderr)
    record("map", "str_ops", times)


# --- plan-45: Integer-key hash path + Map OF List aggregation ---------------

def test_map_intkey():
    n = 5000
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        m = {}
        for i in range(n):
            m[i * 2] = i
        acc = 0
        for i in range(n):
            if i * 2 in m:
                acc += m[i * 2]
            acc += m.get(i * 2 + 1, 0)
        checksum = acc
        times.append(now_ns() - t0)
    print("map_intkey = %d" % checksum, file=sys.stderr)
    record("map", "intkey", times)


def test_map_intchurn():
    # Arena-gated in mfb (plan-44-J); the Python mirror keeps the same tiny counts.
    w = 32
    steps = 64
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        m = {}
        for i in range(w):
            m[i] = i
        acc = 0
        for st in range(steps):
            add_key = w + st
            m[add_key] = add_key
            rem_key = st
            if rem_key in m:
                del m[rem_key]
            acc += m.get(add_key, 0)
        checksum = acc + len(m)
        times.append(now_ns() - t0)
    print("map_intchurn = %d" % checksum, file=sys.stderr)
    record("map", "intchurn", times)


def test_map_listagg():
    n = 2000
    buckets = 64
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        m = {}
        for i in range(n):
            key = i % buckets
            bucket = m.get(key, [])
            bucket = bucket + [i]
            m[key] = bucket
        acc = 0
        for key in range(buckets):
            if key in m:
                bucket = m[key]
                acc += (key + 1) * len(bucket) + sum(bucket)
        checksum = acc
        times.append(now_ns() - t0)
    print("map_listagg = %d" % checksum, file=sys.stderr)
    record("map", "listagg", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_map_set()
    test_map_lookup()
    test_map_int_ops()
    test_map_str_ops()
    test_map_intkey()
    test_map_intchurn()
    test_map_listagg()
