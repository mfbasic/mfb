"""Dispatch-group benchmark (union+MATCH tag dispatch, inline-TRAP recovery).

Mirrors benchmark/mfb/src/dispatch.mfb:
  union  build a perfect binary expression tree (Num/Add/Mul) and evaluate it many
         times, dispatching per node with isinstance (the Python analogue of MATCH
         on the union tag). Arithmetic is mod 1000000007.
  trap   parse a mixed valid/invalid token stream, recovering each failure with
         try/except (the Python analogue of an inline TRAP), so the error path is
         taken 1/4 of the time.
A per-iteration seed is added at every leaf so each full tree evaluation is
distinct (otherwise a C -O2 build hoists the constant eval and the row is noise).
Both checksums (union=212666511, trap=37475000) match the mfb and C references.
"""
import sys

RUN = 1
now_ns = None
record = None

M = 1000000007


def _build_tree(total, internal):
    nodes = []
    for k in range(total):
        if k < internal:
            if k % 2 == 0:
                nodes.append(("add", 2 * k + 1, 2 * k + 2))
            else:
                nodes.append(("mul", 2 * k + 1, 2 * k + 2))
        else:
            nodes.append(("num", (k % 7) + 1))
    return nodes


def test_dispatch_union():
    total, internal = 2047, 1023
    nodes = _build_tree(total, internal)
    evals = 2000
    sys.setrecursionlimit(100000)

    def ev(i, seed):
        n = nodes[i]
        tag = n[0]
        if tag == "num":
            return (n[1] + seed) % M
        if tag == "add":
            return (ev(n[1], seed) + ev(n[2], seed)) % M
        return (ev(n[1], seed) * ev(n[2], seed)) % M

    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for n in range(evals):
            acc = (acc + ev(0, n)) % M
        checksum = acc
        times.append(now_ns() - t0)
    print("dispatch_union = %d" % checksum, file=sys.stderr)
    record("dispatch", "union", times)


def test_dispatch_trap():
    tokens = ["bad" if i % 4 == 0 else str(i) for i in range(1000)]
    passes = 100
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _p in range(passes):
            for tok in tokens:
                try:
                    v = int(tok)
                except ValueError:
                    v = -1
                acc += v
        checksum = acc
        times.append(now_ns() - t0)
    print("dispatch_trap = %d" % checksum, file=sys.stderr)
    record("dispatch", "trap", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_dispatch_union()
    test_dispatch_trap()
