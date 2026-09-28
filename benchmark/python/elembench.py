"""Record- and Float-element collection benchmarks (group `elem`).

Mirrors benchmark/mfb/src/elem.mfb (P1 coverage): the common collection ops
over a list of records (a tuple (id, name, score) — mfb's Item record), a list
of floats, and a dict int -> record. sortBy is `sorted(key=...)` (timsort, stable,
like mfb's sortBy); find is `list.index` (tuple equality, field by field);
filter/transform are list comprehensions building fresh lists; set of a record
builds a fresh tuple (mfb's WITH). Scores are exact dyadics (multiples of 0.25)
so every Float sum is exact and the integer projections match mfb and C.

Expected checksums (run-count independent): rec_get=524258000, rec_set=681148,
rec_sortby=1463331720, rec_find=999000, rec_filter=101400,
rec_transform=240236, flt_sort=1163083625, flt_sum=9999000000,
flt_transform=4994700, map_rec_set=20217, map_rec_get=262129000.
"""
import sys
from functools import reduce

RUN = 1
now_ns = None
record = None

ID, NAME, SCORE = 0, 1, 2


def make_item(i):
    return (i, "item%d" % i, float((i * 37) % 101) * 0.5)


def build_items(n):
    return [make_item(i) for i in range(n)]


def build_floats(n):
    return [float((i * 7919) % n) * 0.25 for i in range(n)]


def score_key(it):
    return it[SCORE]


def add_f(acc, x):
    return acc + x


def _row(name, fn):
    times = []
    checksum = 0
    for _ in range(RUN):
        pre = fn.pre() if hasattr(fn, "pre") else None
        t0 = now_ns()
        checksum = fn(pre)
        times.append(now_ns() - t0)
    print("test_elem_%s = %d" % (name, checksum), file=sys.stderr)
    record("elem", name, times)


def test_rec_get():
    items = build_items(1000)

    def body(_):
        acc = 0
        for _k in range(1000):
            for i in range(1000):
                it = items[i]
                acc += it[ID] + int(it[SCORE])
        return acc
    _row("rec_get", body)


def test_rec_set():
    def body(items):
        for _p in range(100):
            for j in range(1000):
                it = items[j]
                items[j] = (it[ID] + 1, it[NAME], it[SCORE] + 0.5)
        acc = 0
        for j in range(1000):
            it = items[j]
            acc += it[ID] + int(it[SCORE]) + len(it[NAME])
        return acc
    body.pre = lambda: build_items(1000)  # rebuilt untimed each run
    _row("rec_set", body)


def test_rec_sortby():
    items = build_items(1000)

    def body(_):
        acc = 0
        for _k in range(20):
            srt = sorted(items, key=score_key)
            h = 0
            for it in srt:
                h = (h * 31 + it[ID]) % 1000000007
            acc += h
        return acc
    _row("rec_sortby", body)


def test_rec_find():
    items = build_items(1000)
    needle = make_item(999)

    def body(_):
        acc = 0
        for _k in range(1000):
            acc += items.index(needle)
        return acc
    _row("rec_find", body)


def test_rec_filter():
    items = build_items(1000)

    def body(_):
        acc = 0
        for _k in range(200):
            high = [it for it in items if it[SCORE] >= 25.0]
            acc += len(high) + high[0][ID]
        return acc
    _row("rec_filter", body)


def test_rec_transform():
    items = build_items(1000)

    def body(_):
        acc = 0
        for k in range(200):
            scaled = [(it[ID], it[NAME], it[SCORE] * 2.0 + 0.25) for it in items]
            acc += len(scaled) + int(scaled[k][SCORE] * 4.0)
        return acc
    _row("rec_transform", body)


def test_flt_sort():
    xs = build_floats(20000)

    def body(_):
        acc = 0
        for _k in range(5):
            srt = sorted(xs)
            h = 0
            for x in srt:
                h = (h * 31 + int(x * 4.0)) % 1000000007
            acc += h
        return acc
    _row("flt_sort", body)


def test_flt_sum():
    xs = build_floats(10000)

    def body(_):
        acc = 0
        for _k in range(200):
            total = reduce(add_f, xs, 0.0)
            acc += int(total * 4.0)
        return acc
    _row("flt_sum", body)


def test_flt_transform():
    xs = build_floats(10000)

    def body(_):
        acc = 0
        for k in range(200):
            ys = [x * 1.5 + 0.25 for x in xs]
            acc += len(ys) + int(ys[k] * 8.0)
        return acc
    _row("flt_transform", body)


def test_map_rec_set():
    items = build_items(1000)

    def body(_):
        acc = 0
        for k in range(10):
            m = {}
            for i in range(1000):
                m[i] = items[i]
            for i in range(1000):
                it = items[i]
                m[i] = (it[ID], it[NAME], it[SCORE] + 1.0)
            last = m[999 - k]
            acc += len(m) + last[ID] + int(last[SCORE])
        return acc
    _row("map_rec_set", body)


def test_map_rec_get():
    items = build_items(1000)
    m = {}
    for i in range(1000):
        m[i] = items[i]

    def body(_):
        acc = 0
        for _k in range(500):
            for i in range(1000):
                it = m[i]
                acc += it[ID] + int(it[SCORE])
        return acc
    _row("map_rec_get", body)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_rec_get()
    test_rec_set()
    test_rec_sortby()
    test_rec_find()
    test_rec_filter()
    test_rec_transform()
    test_flt_sort()
    test_flt_sum()
    test_flt_transform()
    test_map_rec_set()
    test_map_rec_get()
