"""serialize-group benchmark (json::/csv:: encode direction).

Mirrors benchmark/mfb/src/serialize.mfb: json stringify, json parse->stringify
round-trip, and csv stringify. The checksum is the *length* of the emitted text
(accumulated over the reps) — order-independent, so it matches across languages
even though json object members may be emitted in a different order. The canonical
inputs use only ASCII strings and integers (no '/', no fractional numbers), so
Python's compact json.dumps produces the same length as mfb's json::stringify.

csv is hand-rolled to mfb's csv::stringify rules (quote a field iff it holds a
comma / quote / CR / LF; double interior quotes; join fields with ',', rows with
LF, no trailing newline). csv.writer is not used here because it appends a line
terminator after the final row, which would make the length differ from mfb's
no-trailing-newline output by one.
"""
import json
import sys

RUN = 1
now_ns = None
record = None

_JSON_TEXT = ('{"name":"benchmark","version":3,"tags":["alpha","beta","gamma"],'
              '"nested":{"count":42,"label":"node","values":[1,2,3,4,5]},'
              '"active":1}')

_GRID = [["id", "name", "note"], ["1", "Grace", "Hop,per"],
         ["2", "Ada", "plain"], ["3", "Alan", 'say"hi']]


def _json_stringify(value):
    return json.dumps(value, separators=(",", ":"))


def _csv_stringify(grid):
    def field(s):
        if any(c in s for c in (",", '"', "\r", "\n")):
            return '"' + s.replace('"', '""') + '"'
        return s
    return "\n".join(",".join(field(c) for c in row) for row in grid)


def test_serialize_json():
    tree = json.loads(_JSON_TEXT)
    reps = 4                 # TODO(plan-64-A): raise to 200
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(reps):
            acc += len(_json_stringify(tree))
        checksum = acc
        times.append(now_ns() - t0)
    print("serialize_json = %d" % checksum, file=sys.stderr)
    record("serialize", "json", times)


def test_serialize_roundtrip():
    reps = 4                 # TODO(plan-64-A): raise to 100
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(reps):
            tree = json.loads(_JSON_TEXT)
            acc += len(_json_stringify(tree))
        checksum = acc
        times.append(now_ns() - t0)
    print("serialize_roundtrip = %d" % checksum, file=sys.stderr)
    record("serialize", "roundtrip", times)


def test_serialize_csv():
    reps = 4                 # TODO(plan-64-A): raise to 200
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(reps):
            acc += len(_csv_stringify(_GRID))
        checksum = acc
        times.append(now_ns() - t0)
    print("serialize_csv = %d" % checksum, file=sys.stderr)
    record("serialize", "csv", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_serialize_json()
    test_serialize_roundtrip()
    test_serialize_csv()
