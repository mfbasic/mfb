"""Map benchmark matrix — Python peer for mfb's mapmatrix.mfb (Fixed/Dynamic).

Mirrors the mfb map value-matrix plain groups plus the key-hash pair, row for row
and size for size (see benchmark/mfb/gen_map.py for the sizes). C/Python do not
need the Record/State container variants (bug-430 is an mfb value-semantics
story); they carry only the plain Fixed/Dynamic element axis so the cross-language
table lines up. Checksums are count/sum based (order-independent), so they match
mfb without matching key iteration order.

  map (Fixed)        Integer key -> Integer value
  map (Dynamic)      Integer key -> String  value
  map (key-Fixed)    Integer key -> Integer value  (same workload as Fixed)
  map (key-Dynamic)  String  key -> Integer value
"""
import sys

RUN = 1
now_ns = None
record = None

INT = dict(set_n=300, rem_n=300, ro_n=1000, prod_n=300, prod_sh=150,
           k_get=100, k_keys=200, k_prod=20)
STR = dict(set_n=400, rem_n=400, ro_n=200, prod_n=100, prod_sh=50,
           k_get=50, k_keys=100, k_prod=15)

# label, fn-prefix, key type, value type, sizes
GROUPS = [
    ("Fixed", "mf", "int", "int", INT),
    ("Dynamic", "md", "int", "str", STR),
    ("key-Fixed", "mkf", "int", "int", INT),
    ("key-Dynamic", "mkd", "str", "int", STR),
]


def _key(kty, i):
    return i if kty == "int" else "k" + str(i)


def _val(vty, i):
    return i if vty == "int" else "v" + str(i)


def _agg(vty, v):
    return v if vty == "int" else len(v)


def _mapfn(vty, v):
    return v + v if vty == "int" else "[" + v + "]"


def _build(kty, vty, n, sh=0):
    return {_key(kty, i + sh): _val(vty, i + sh) for i in range(n)}


def _time(fn):
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        checksum = fn()
        times.append(now_ns() - t0)
    return times, checksum


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    for label, pfx, kty, vty, S in GROUPS:
        group = "map (%s)" % label

        def emit(op, fn, setup=None):
            # setup() builds once outside timing (read-only ops); fn() is timed.
            base = setup() if setup else None
            times, checksum = _time(lambda: fn(base))
            print("test_%s_%s = %d" % (pfx, op, checksum), file=sys.stderr)
            record(group, op, times)

        def op_set(_):
            m = {}
            for i in range(S["set_n"]):
                m[_key(kty, i)] = _val(vty, i)
            return len(m)
        emit("set", op_set)

        def op_get(base):
            acc = 0
            for _ in range(S["k_get"]):
                for i in range(S["ro_n"]):
                    acc += _agg(vty, base[_key(kty, i)])
            return acc
        emit("get", op_get, lambda: _build(kty, vty, S["ro_n"]))

        def op_getor(base):
            default = 0 if vty == "int" else ""
            acc = 0
            for _ in range(S["k_get"]):
                for i in range(S["ro_n"]):
                    acc += _agg(vty, base.get(_key(kty, i), default))
            return acc
        emit("getOr", op_getor, lambda: _build(kty, vty, S["ro_n"]))

        def op_haskey(base):
            acc = 0
            for _ in range(S["k_get"]):
                for i in range(S["ro_n"]):
                    if _key(kty, i) in base:
                        acc += 1
            return acc
        emit("hasKey", op_haskey, lambda: _build(kty, vty, S["ro_n"]))

        def op_removekey(_):
            m = _build(kty, vty, S["rem_n"])
            cnt = 0
            for i in range(S["rem_n"]):
                m.pop(_key(kty, i), None)
                cnt += 1
            return cnt
        emit("removeKey", op_removekey)

        def op_keys(base):
            acc = 0
            for _ in range(S["k_keys"]):
                acc += len(list(base.keys()))
            return acc
        emit("keys", op_keys, lambda: _build(kty, vty, S["ro_n"]))

        def op_values(base):
            acc = 0
            for _ in range(S["k_keys"]):
                acc += len(list(base.values()))
            return acc
        emit("values", op_values, lambda: _build(kty, vty, S["ro_n"]))

        def op_mapvalues(base):
            acc = 0
            for _ in range(S["k_prod"]):
                acc += len({k: _mapfn(vty, v) for k, v in base.items()})
            return acc
        emit("mapValues", op_mapvalues, lambda: _build(kty, vty, S["prod_n"]))

        def op_merge(base):
            other = _build(kty, vty, S["prod_n"], S["prod_sh"])
            acc = 0
            for _ in range(S["k_prod"]):
                merged = dict(base)
                merged.update(other)
                acc += len(merged)
            return acc
        emit("merge", op_merge, lambda: _build(kty, vty, S["prod_n"]))
