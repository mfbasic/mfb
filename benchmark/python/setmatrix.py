"""Set benchmark matrix — Python peer for mfb's setops.mfb plain groups.

Mirrors mfb set (Fixed) / set (Dynamic) row for row and size for size (see
benchmark/mfb/gen_set.py). C/Python carry only the plain element axis — the
Record/State container variants are an mfb value-semantics (bug-430) story.
Checksums are count based (order-independent), so they match mfb regardless of
set iteration order.

  set (Fixed)    Set of Integer
  set (Dynamic)  Set of String
"""
import sys

RUN = 1
now_ns = None
record = None

INT = dict(add_n=300, rem_n=300, ro_n=1000, alg_n=300, alg_sh=150,
           k_contains=100, k_tolist=200, k_alg=20, k_pred=300)
STR = dict(add_n=400, rem_n=400, ro_n=200, alg_n=100, alg_sh=50,
           k_contains=50, k_tolist=100, k_alg=15, k_pred=60)

GROUPS = [("Fixed", "sf", "int", INT), ("Dynamic", "sd", "str", STR)]


def _el(ty, i):
    return i if ty == "int" else "s" + str(i)


def _build(ty, n, sh=0):
    return {_el(ty, i + sh) for i in range(n)}


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    for label, pfx, ty, S in GROUPS:
        group = "set (%s)" % label

        def emit(op, fn, setup=None):
            arg = setup() if setup else None
            times = []
            checksum = 0
            for _ in range(RUN):
                t0 = now_ns()
                checksum = fn(arg)
                times.append(now_ns() - t0)
            print("test_%s_%s = %d" % (pfx, op, checksum), file=sys.stderr)
            record(group, op, times)

        emit("add", lambda _: len({_el(ty, i) for i in range(S["add_n"])}))

        def op_remove(_):
            s = _build(ty, S["rem_n"])
            cnt = 0
            for i in range(S["rem_n"]):
                s.discard(_el(ty, i))
                cnt += 1
            return cnt
        emit("remove", op_remove)

        def op_contains(base):
            acc = 0
            for _ in range(S["k_contains"]):
                for i in range(S["ro_n"]):
                    if _el(ty, i) in base:
                        acc += 1
            return acc
        emit("contains", op_contains, lambda: _build(ty, S["ro_n"]))

        def op_tolist(base):
            acc = 0
            for _ in range(S["k_tolist"]):
                acc += len(list(base))
            return acc
        emit("toList", op_tolist, lambda: _build(ty, S["ro_n"]))

        def op_toset(base):
            acc = 0
            for _ in range(S["k_alg"]):
                acc += len(set(list(base)))
            return acc
        emit("toSet", op_toset, lambda: _build(ty, S["alg_n"]))

        def alg(fn):
            def run(base):
                other = _build(ty, S["alg_n"], S["alg_sh"])
                acc = 0
                for _ in range(S["k_alg"]):
                    acc += len(fn(base, other))
                return acc
            return run
        emit("union", alg(lambda a, b: a | b), lambda: _build(ty, S["alg_n"]))
        emit("intersection", alg(lambda a, b: a & b), lambda: _build(ty, S["alg_n"]))
        emit("difference", alg(lambda a, b: a - b), lambda: _build(ty, S["alg_n"]))
        emit("symmetricDifference", alg(lambda a, b: a ^ b), lambda: _build(ty, S["alg_n"]))

        # plan-121-E: these predicates used to run against a partially
        # overlapping `other` and were FALSE on every call, so each language
        # early-exited at whatever point ITS OWN iteration order met the single
        # counterexample -- C walked its hash slot array and stopped after 2-3
        # probes where mfb, walking in entry order, did 501. Same answer, 250x
        # the work, and the row reported iteration order rather than throughput.
        #
        # Now every predicate is TRUE, which forces a FULL scan everywhere: there
        # is no counterexample to meet early or late. `other` is chosen per
        # predicate to make that so, at the same `alg_n` size all three languages
        # already shared.
        def _pred(fn, other_fn):
            def run(base):
                other = other_fn()
                acc = 0
                for _ in range(S["k_pred"]):
                    if fn(base, other):
                        acc += 1
                return acc
            return run

        # subset/superset: compare the set with itself -> TRUE, full scan.
        def pred_same(fn):
            return _pred(fn, lambda: _build(ty, S["alg_n"]))

        # disjoint: shift clear of the base by its own size -> TRUE, full scan.
        def pred_disj(fn):
            return _pred(fn, lambda: _build(ty, S["alg_n"], S["alg_n"]))

        emit("isSubset", pred_same(lambda a, b: a <= b), lambda: _build(ty, S["alg_n"]))
        emit("isSuperset", pred_same(lambda a, b: a >= b), lambda: _build(ty, S["alg_n"]))
        emit("isDisjoint", pred_disj(lambda a, b: a.isdisjoint(b)), lambda: _build(ty, S["alg_n"]))
