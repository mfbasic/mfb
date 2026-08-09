"""Chained collection HOF pipeline benchmarks (group `pipeline`).

Mirrors benchmark/mfb/src/pipeline.mfb (plan-87 Theme 2). Each row materializes
the same intermediate lists mfb's filter/transform/reduce/groupBy/mapValues/
values chain builds, so the cross-language checksums match.

Expected checksums: int=74990000, groupagg=49995000, str=412.
"""
import sys
from functools import reduce

RUN = 1
now_ns = None
record = None

PIPE_N = 10000
PIPE_K = 7


def test_pipeline_int():
    data = list(range(PIPE_N))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        total = 0
        for _rep in range(200):
            evens = [x for x in data if x % 2 == 0]       # filter
            tripled = [x * 3 + 1 for x in evens]          # transform
            total = reduce(lambda acc, x: acc + x, tripled, 0)  # reduce
        checksum = total
        times.append(now_ns() - t0)
    print("pipeline_int = %d" % checksum, file=sys.stderr)
    record("pipeline", "int", times)


def test_pipeline_groupagg():
    data = list(range(PIPE_N))
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        total = 0
        for _rep in range(100):
            buckets = {}                                   # groupBy n MOD K
            for n in data:
                buckets.setdefault(n % PIPE_K, []).append(n)
            sums = {k: reduce(lambda acc, x: acc + x, v, 0)  # mapValues(reduce)
                    for k, v in buckets.items()}
            bucket_sums = list(sums.values())             # values
            total = reduce(lambda acc, x: acc + x, bucket_sums, 0)  # reduce
        checksum = total
        times.append(now_ns() - t0)
    print("pipeline_groupagg = %d" % checksum, file=sys.stderr)
    record("pipeline", "groupagg", times)


def test_pipeline_str():
    data = ["" if i % 7 == 0 else "row%dValue" % i for i in range(50)]
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        total = 0
        for _rep in range(20):
            kept = [s for s in data if s]                 # filter non-empty
            upped = [s.upper() for s in kept]             # transform
            total = reduce(lambda acc, s: acc + len(s), upped, 0)  # reduce
        checksum = total
        times.append(now_ns() - t0)
    print("pipeline_str = %d" % checksum, file=sys.stderr)
    record("pipeline", "str", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_pipeline_int()
    test_pipeline_groupagg()
    test_pipeline_str()
