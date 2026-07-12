import sys

RUN = 1
now_ns = None
record = None
forEachAcc = 0


def test_list_append():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        nums = []
        for i in range(1000):
            nums.append(i)
        checksum = len(nums)
        times.append(now_ns() - t0)
    print("list_append = %d" % checksum, file=sys.stderr)
    record("list", "append", times)


def test_list_append_batch():
    ten = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        nums = []
        for _i in range(100):
            nums.extend(ten)
        checksum = len(nums)
        times.append(now_ns() - t0)
    print("list_append_batch = %d" % checksum, file=sys.stderr)
    record("list", "append_batch", times)


def test_list_prepend():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        nums = []
        for i in range(1000):
            nums.insert(0, i)
        checksum = len(nums)
        times.append(now_ns() - t0)
    print("list_prepend = %d" % checksum, file=sys.stderr)
    record("list", "prepend", times)


def test_list_copy():
    strs = [str(i) for i in range(1000)]
    recs = [(i, str(i)) for i in range(1000)]
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _i in range(1000):
            c = list(strs)
            acc += len(c)
        for _i in range(1000):
            cr = list(recs)
            acc += len(cr)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_copy = %d" % checksum, file=sys.stderr)
    record("list", "copy", times)


def test_list_distinct():
    times = []
    checksum = 0
    for _ in range(RUN):
        nums = [i % 1000 for i in range(5000)]
        t0 = now_ns()
        unique = []
        for v in nums:
            if v not in unique:
                unique.append(v)
        checksum = len(unique)
        times.append(now_ns() - t0)
    print("list_distinct = %d" % checksum, file=sys.stderr)
    record("list", "distinct", times)


def test_list_groupby():
    times = []
    checksum = 0
    for _ in range(RUN):
        nums = list(range(2000))
        t0 = now_ns()
        groups = {}
        for v in nums:
            groups.setdefault(v % 100, []).append(v)
        checksum = len(groups)
        times.append(now_ns() - t0)
    print("list_groupby = %d" % checksum, file=sys.stderr)
    record("list", "groupby", times)


def test_list_set():
    times = []
    checksum = 0
    for _ in range(RUN):
        nums = list(range(200))
        t0 = now_ns()
        for _pass in range(10):
            for j in range(200):
                nums[j] = nums[j] + 1
        times.append(now_ns() - t0)
        checksum = sum(nums)
    print("list_set = %d" % checksum, file=sys.stderr)
    record("list", "set", times)


def test_list_sort():
    import random
    times = []
    checksum = 0
    for _ in range(RUN):
        base = [random.randint(0, 1000000) for _ in range(50)]
        t0 = now_ns()
        s = sorted(base)
        times.append(now_ns() - t0)
        checksum = s[0]
    print("list_sort = %d" % checksum, file=sys.stderr)
    record("list", "sort", times)


forEachAcc = 0


def test_list_all():
    pos = list(range(1, 1001))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(200):
            if all(x > 0 for x in pos):
                acc += 1
        checksum = acc
        times.append(now_ns() - t0)
    print("list_all = %d" % checksum, file=sys.stderr)
    record("list", "all", times)


def test_list_any():
    neg = [-i for i in range(1, 1001)]
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(200):
            if not any(x > 0 for x in neg):
                acc += 1
        checksum = acc
        times.append(now_ns() - t0)
    print("list_any = %d" % checksum, file=sys.stderr)
    record("list", "any", times)


def test_list_chunks():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(200):
            chunks = [base[i:i + 10] for i in range(0, len(base), 10)]
            acc += len(chunks)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_chunks = %d" % checksum, file=sys.stderr)
    record("list", "chunks", times)


def test_list_contains():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(500):
            if 1000 not in base:
                acc += 1
        checksum = acc
        times.append(now_ns() - t0)
    print("list_contains = %d" % checksum, file=sys.stderr)
    record("list", "contains", times)


def test_list_drop():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(500):
            dropped = base[500:]
            acc += len(dropped)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_drop = %d" % checksum, file=sys.stderr)
    record("list", "drop", times)


def test_list_filter():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(200):
            matched = [x for x in base if x % 2 == 0]
            acc += len(matched)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_filter = %d" % checksum, file=sys.stderr)
    record("list", "filter", times)


def test_list_find():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(500):
            acc += base.index(999)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_find = %d" % checksum, file=sys.stderr)
    record("list", "find", times)


def test_list_findIndex():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(500):
            idx = -1
            for i in range(len(base)):
                if base[i] >= 999:
                    idx = i
                    break
            acc += idx
        checksum = acc
        times.append(now_ns() - t0)
    print("list_findIndex = %d" % checksum, file=sys.stderr)
    record("list", "findIndex", times)


def test_list_findLastIndex():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(500):
            idx = -1
            for i in range(len(base)):
                if base[i] <= 5:
                    idx = i
            acc += idx
        checksum = acc
        times.append(now_ns() - t0)
    print("list_findLastIndex = %d" % checksum, file=sys.stderr)
    record("list", "findLastIndex", times)


def test_list_flatten():
    nested = [list(range(10)) for _ in range(100)]
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(200):
            flat = [x for row in nested for x in row]
            acc += len(flat)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_flatten = %d" % checksum, file=sys.stderr)
    record("list", "flatten", times)


def test_list_forEach():
    global forEachAcc
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        forEachAcc = 0
        for _k in range(200):
            for x in base:
                forEachAcc += x
        checksum = forEachAcc
        times.append(now_ns() - t0)
    print("list_forEach = %d" % checksum, file=sys.stderr)
    record("list", "forEach", times)


def test_list_get():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _pass in range(100):
            for i in range(1000):
                acc += base[i]
        checksum = acc
        times.append(now_ns() - t0)
    print("list_get = %d" % checksum, file=sys.stderr)
    record("list", "get", times)


def test_list_getOr():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _pass in range(100):
            for i in range(1000):
                acc += base[i]
        checksum = acc
        times.append(now_ns() - t0)
    print("list_getOr = %d" % checksum, file=sys.stderr)
    record("list", "getOr", times)


def test_list_insert():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        nums = []
        for i in range(1000):
            nums.insert(len(nums) // 2, i)
        checksum = len(nums)
        times.append(now_ns() - t0)
    print("list_insert = %d" % checksum, file=sys.stderr)
    record("list", "insert", times)


def test_list_mid():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(500):
            m = base[250:250 + 500]
            acc += len(m)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_mid = %d" % checksum, file=sys.stderr)
    record("list", "mid", times)


def test_list_partition():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(200):
            matched = []
            unmatched = []
            for x in base:
                if x % 2 == 0:
                    matched.append(x)
                else:
                    unmatched.append(x)
            acc += len(matched)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_partition = %d" % checksum, file=sys.stderr)
    record("list", "partition", times)


def test_list_reduce():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(500):
            total = 0
            for x in base:
                total += x
            acc += total
        checksum = acc
        times.append(now_ns() - t0)
    print("list_reduce = %d" % checksum, file=sys.stderr)
    record("list", "reduce", times)


def test_list_reduceRight():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(500):
            total = 0
            for x in reversed(base):
                total += x
            acc += total
        checksum = acc
        times.append(now_ns() - t0)
    print("list_reduceRight = %d" % checksum, file=sys.stderr)
    record("list", "reduceRight", times)


def test_list_removeAt():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        nums = list(base)
        t0 = now_ns()
        count = 0
        while nums:
            nums.pop(0)
            count += 1
        checksum = count
        times.append(now_ns() - t0)
    print("list_removeAt = %d" % checksum, file=sys.stderr)
    record("list", "removeAt", times)


def test_list_replace():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(200):
            replaced = [500 if x == 500 else x for x in base]
            acc += len(replaced)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_replace = %d" % checksum, file=sys.stderr)
    record("list", "replace", times)


def test_list_sortBy():
    base2 = list(range(500))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(200):
            s = sorted(base2, key=lambda n: -n)
            acc += s[0]
        checksum = acc
        times.append(now_ns() - t0)
    print("list_sortBy = %d" % checksum, file=sys.stderr)
    record("list", "sortBy", times)


def test_list_sum():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(1000):
            acc += sum(base)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_sum = %d" % checksum, file=sys.stderr)
    record("list", "sum", times)


def test_list_take():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(500):
            t = base[:500]
            acc += len(t)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_take = %d" % checksum, file=sys.stderr)
    record("list", "take", times)


def test_list_transform():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(200):
            mapped = [x + x for x in base]
            acc += len(mapped)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_transform = %d" % checksum, file=sys.stderr)
    record("list", "transform", times)


def test_list_window():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(100):
            windows = [base[i:i + 10] for i in range(len(base) - 10 + 1)]
            acc += len(windows)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_window = %d" % checksum, file=sys.stderr)
    record("list", "window", times)


def test_list_zip():
    base = list(range(1000))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(100):
            zipped = list(zip(base, base))
            acc += len(zipped)
        checksum = acc
        times.append(now_ns() - t0)
    print("list_zip = %d" % checksum, file=sys.stderr)
    record("list", "zip", times)


# ===================================================================== #
# GROUP: liststr  (collections:: list ops over a List OF String)         #
# Mirrors the liststr rows in benchmark/mfb/src/list.mfb. len() over ASCII #
# strings counts code points == Unicode scalars, so the checksums line up. #
# ===================================================================== #

strForEachAcc = 0


def _build_str_range(n):
    return ["s" + str(i) for i in range(n)]


def _build_str_dup(n, distinct):
    return ["d" + str(i - (i // distinct) * distinct) for i in range(n)]


def _build_str_nested(outer, inner):
    row = _build_str_range(inner)
    return [row for _ in range(outer)]


# append, prepend, insert, set, removeAt, get, getOr, contains
def test_liststr_build():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        nums = []
        for i in range(500):
            nums.append("a" + str(i))
        for i in range(100):
            nums.insert(0, "p" + str(i))
        for i in range(100):
            nums.insert(len(nums) // 2, "m" + str(i))
        acc = 0
        for i in range(200):
            v = nums[i]
            nums[i] = v + "!"
            acc += len(nums[i] if i < len(nums) else "")
        if "a10!" in nums:
            acc += 1
        for _i in range(100):
            nums.pop(0)
        checksum = acc + len(nums)
        times.append(now_ns() - t0)
    print("liststr_build = %d" % checksum, file=sys.stderr)
    record("liststr", "build", times)


# find, findIndex, findLastIndex, all, any
def test_liststr_query():
    base = _build_str_range(1000)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(200):
            acc += base.index("s999")
            acc += next((i for i, s in enumerate(base) if len(s) >= 4), -1)
            last = -1
            for i, s in enumerate(base):
                if len(s) <= 2:
                    last = i
            acc += last
            if all(len(s) > 0 for s in base):
                acc += 1
            if any(len(s) <= 2 for s in base):
                acc += 1
        checksum = acc
        times.append(now_ns() - t0)
    print("liststr_query = %d" % checksum, file=sys.stderr)
    record("liststr", "query", times)


# filter, transform, reduce, reduceRight, forEach, partition
def test_liststr_hof():
    global strForEachAcc
    base = _build_str_range(100)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        strForEachAcc = 0
        for _k in range(200):
            acc += len([s for s in base if len(s) <= 2])
            acc += len(["[" + s + "]" for s in base])
            reduced = "".join(base)
            acc += len(reduced)
            acc += len(reduced)                       # reduceRight, same length
            for s in base:
                strForEachAcc += len(s)
            matched = [s for s in base if len(s) <= 2]
            acc += len(matched)
        checksum = acc + strForEachAcc
        times.append(now_ns() - t0)
    print("liststr_hof = %d" % checksum, file=sys.stderr)
    record("liststr", "hof", times)


# sort, sortBy, distinct, take, drop, mid, chunks, window, zip, flatten, replace
# base list and k kept small to match the mfb coverage row, which must stay
# tiny to dodge a runtime arena mixed-transient-churn slowdown on String
# sort/window (see benchmark/mfb/src/list.mfb and the README).
def test_liststr_reshape():
    base = _build_str_range(40)
    dupbase = _build_str_dup(80, 40)
    nested = _build_str_nested(15, 10)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _k in range(3):
            acc += len(sorted(base)[0])
            acc += len(sorted(base, key=len)[0])
            acc += len(dict.fromkeys(dupbase))
            acc += len(base[:20])
            acc += len(base[20:])
            acc += len(base[10:10 + 20])
            acc += len([base[i:i + 10] for i in range(0, len(base), 10)])
            acc += len([base[i:i + 10] for i in range(len(base) - 10 + 1)])
            acc += len(list(zip(base, base)))
            acc += len([x for row in nested for x in row])
            acc += len(["S5" if x == "s5" else x for x in base])
        checksum = acc
        times.append(now_ns() - t0)
    print("liststr_reshape = %d" % checksum, file=sys.stderr)
    record("liststr", "reshape", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_list_append(); test_list_append_batch(); test_list_prepend(); test_list_copy()
    test_list_distinct(); test_list_groupby(); test_list_set(); test_list_sort()
    test_list_all(); test_list_any(); test_list_chunks(); test_list_contains()
    test_list_drop(); test_list_filter(); test_list_find(); test_list_findIndex()
    test_list_findLastIndex(); test_list_flatten(); test_list_forEach(); test_list_get()
    test_list_getOr(); test_list_insert(); test_list_mid(); test_list_partition()
    test_list_reduce(); test_list_reduceRight(); test_list_removeAt(); test_list_replace()
    test_list_sortBy(); test_list_sum(); test_list_take(); test_list_transform()
    test_list_window(); test_list_zip()
    # liststr group (String element type)
    test_liststr_build(); test_liststr_query(); test_liststr_hof(); test_liststr_reshape()
