"""List benchmark matrix — Python peer for mfb's list.mfb plain groups.

Mirrors mfb list (Fixed) / list (Dynamic) row for row (see benchmark/mfb/
gen_list.py). Same sizes for both element types; only the element differs
(Integer vs "s"+i String). C/Python carry only the plain element axis — the
Record/State variants are an mfb value-semantics (bug-430) story. Replaces the
old `list` + `liststr` groups. Most checksums are count/sum based; the sort
adaptivity rows use an order-sensitive polynomial hash identical to mfb.
"""
import sys

RUN = 1
now_ns = None
record = None

GROUPS = [("Fixed", "lf", "int"), ("Dynamic", "ld", "str")]


def el(ty, i):
    return i if ty == "int" else "s" + str(i)


def rng(ty, n):
    return [el(ty, i) for i in range(n)]


def _pad6(n):
    s = str(n)
    return "0" * (6 - len(s)) + s if len(s) < 6 else s


def _sorthash_int(a):
    acc = 0
    for v in a:
        acc = (acc * 31 + v) % 1000000007
    return acc


def _sorthash_str(a):
    acc = 0
    for v in a:
        acc = (acc * 31 + len(v)) % 1000000007
    return acc


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    for label, pfx, ty in GROUPS:
        group = "list (%s)" % label
        is_int = ty == "int"

        def emit(op, fn):
            times = []
            checksum = 0
            for _ in range(RUN):
                t0 = now_ns()
                checksum = fn()
                times.append(now_ns() - t0)
            print("test_%s_%s = %d" % (pfx, op, checksum), file=sys.stderr)
            record(group, op, times)

        # predicates / helpers per element type
        if is_int:
            isPos = lambda n: n > 0
            isEven = lambda n: n % 2 == 0
            geHi = lambda n: n >= 999
            leLo = lambda n: n <= 5
            addf = lambda a, n: a + n
            transf = lambda n: n + n
            sortkey = lambda n: -n
            gkey = lambda n: n % 100
            r_init = 0
            r_len = lambda v: v            # reduce result agg
            g_len = lambda v: v            # get/find agg
        else:
            isPos = lambda s: len(s) > 0        # strNonEmpty (all pass)
            isEven = lambda s: len(s) <= 2      # strIsShort (filter)
            geHi = lambda s: len(s) >= 4        # strIsLong (findIndex)
            leLo = lambda s: len(s) <= 2        # strIsShort (findLastIndex)
            addf = lambda a, s: a + s
            transf = lambda s: "[" + s + "]"
            sortkey = lambda s: len(s)
            gkey = lambda s: len(s)
            r_init = ""
            r_len = lambda v: len(v)
            g_len = lambda v: len(v)

        pos = [el(ty, i) for i in range(1, 1001)] if is_int else rng(ty, 1000)
        neg = [-(i) for i in range(1, 1001)] if is_int else rng(ty, 1000)
        base1k = rng(ty, 1000)

        # --- accumulate-from-empty ---------------------------------------
        def op_append():
            xs = []
            for i in range(1000):
                xs.append(el(ty, i))
            return len(xs)
        emit("append", op_append)

        ten = list(range(10)) if is_int else ["a" + str(i) for i in range(10)]

        def op_append_batch():
            xs = []
            for _ in range(100):
                xs.extend(ten)
            return len(xs)
        emit("append_batch", op_append_batch)

        def op_prepend():
            xs = []
            for i in range(1000):
                xs.insert(0, el(ty, i))
            return len(xs)
        emit("prepend", op_prepend)

        def op_insert():
            xs = []
            for i in range(1000):
                xs.insert(len(xs) // 2, el(ty, i))
            return len(xs)
        emit("insert", op_insert)

        # --- read-only ---------------------------------------------------
        emit("copy", lambda: sum(len(list(base1k)) for _ in range(1000)))

        dup = [el(ty, i % 1000) for i in range(5000)]
        emit("distinct", lambda: len(list(dict.fromkeys(dup))))

        g2000 = rng(ty, 2000)

        def op_groupby():
            g = {}
            for v in g2000:
                g.setdefault(gkey(v), []).append(v)
            return len(g)
        emit("groupby", op_groupby)

        def op_set():
            xs = rng(ty, 200)
            for _ in range(10):
                for j in range(200):
                    v = xs[j]
                    xs[j] = v + 1 if is_int else v + "!"
            return sum((xs[j] if is_int else len(xs[j])) for j in range(200))
        emit("set", op_set)

        rand50 = rng(ty, 50)          # deterministic; sort checksum is len
        emit("sort", lambda: len(sorted(rand50)))

        asc = list(range(20000)) if is_int else [_pad6(i) for i in range(20000)]
        desc = list(range(19999, -1, -1)) if is_int else [_pad6(19999 - i) for i in range(20000)]
        scr = [(i * 7919) % 20000 for i in range(20000)] if is_int else [_pad6((i * 7919) % 20000) for i in range(20000)]
        sh = _sorthash_int if is_int else _sorthash_str
        emit("sort_asc", lambda: sh(sorted(asc)))
        emit("sort_desc", lambda: sh(sorted(desc)))
        emit("sort_rand", lambda: sh(sorted(scr)))

        emit("all", lambda: sum(1 for _ in range(200) if all(isPos(v) for v in pos)))
        emit("any", lambda: sum(1 for _ in range(200) if not any(isPos(v) for v in neg)))
        emit("chunks", lambda: sum(len([base1k[i:i + 10] for i in range(0, len(base1k), 10)]) for _ in range(200)))
        target_contains = 1000 if is_int else "s1000"
        emit("contains", lambda: sum(1 for _ in range(500) if target_contains not in base1k))
        emit("drop", lambda: sum(len(base1k[500:]) for _ in range(500)))
        emit("filter", lambda: sum(len([v for v in base1k if isEven(v)]) for _ in range(200)))
        find_t = 999 if is_int else "s999"

        def _find(xs, t):
            try:
                return xs.index(t)
            except ValueError:
                return -1
        emit("find", lambda: sum(_find(base1k, find_t) for _ in range(500)))

        def _findidx(xs, p):
            for i, v in enumerate(xs):
                if p(v):
                    return i
            return -1
        emit("findIndex", lambda: sum(_findidx(base1k, geHi) for _ in range(500)))

        def _findlast(xs, p):
            for i in range(len(xs) - 1, -1, -1):
                if p(xs[i]):
                    return i
            return -1
        emit("findLastIndex", lambda: sum(_findlast(base1k, leLo) for _ in range(500)))

        nested = [rng(ty, 10) for _ in range(100)]
        emit("flatten", lambda: sum(len([x for row in nested for x in row]) for _ in range(200)))

        def op_foreach():
            acc = 0
            for _ in range(200):
                for v in base1k:
                    acc += v if is_int else len(v)
            return acc
        emit("forEach", op_foreach)

        emit("get", lambda: sum(g_len(base1k[i]) for _ in range(100) for i in range(1000)))
        emit("getOr", lambda: sum(g_len(base1k[i]) for _ in range(100) for i in range(1000)))
        emit("mid", lambda: sum(len(base1k[250:750]) for _ in range(500)))

        def op_partition():
            acc = 0
            for _ in range(200):
                m = [v for v in base1k if isEven(v)]
                acc += len(m)
            return acc
        emit("partition", op_partition)

        def _reduce(xs):
            a = r_init
            for v in xs:
                a = addf(a, v)
            return a
        emit("reduce", lambda: sum(r_len(_reduce(base1k)) for _ in range(500)))
        emit("reduceRight", lambda: sum(r_len(_reduce(list(reversed(base1k)))) for _ in range(500)))

        def op_removeat():
            xs = list(base1k)
            cnt = 0
            while xs:
                xs.pop(0)
                cnt += 1
            return cnt
        emit("removeAt", op_removeat)

        rep_a = 500 if is_int else "s5"
        rep_b = 500 if is_int else "S5"
        emit("replace", lambda: sum(len([rep_b if v == rep_a else v for v in base1k]) for _ in range(200)))

        base500 = rng(ty, 500)
        emit("sortBy", lambda: sum(g_len(sorted(base500, key=sortkey)[0]) for _ in range(200)))

        if is_int:
            emit("sum", lambda: sum(sum(base1k) for _ in range(1000)))

        emit("take", lambda: sum(len(base1k[:500]) for _ in range(500)))
        emit("transform", lambda: sum(len([transf(v) for v in base1k]) for _ in range(200)))
        emit("window", lambda: sum(len([base1k[i:i + 10] for i in range(len(base1k) - 9)]) for _ in range(100)))
        emit("zip", lambda: sum(len(list(zip(base1k, base1k))) for _ in range(100)))
