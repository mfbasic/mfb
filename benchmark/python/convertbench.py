"""In-memory number-conversion benchmarks (group `convert`).

Mirrors benchmark/mfb/src/convert.mfb (plan-87 Theme 3): a pure parse+render
loop with no file IO. Python peers use str/int and "%.6f"/float in place of
toString/toInt and toString(_,6)/toFloat.

Expected checksums: int=5000438890, float=624993751111120. The float row uses
v = i/8 (an exact dyadic rational), so the "%.6f" rendering is exact and
unambiguous across the three languages. The value fold rounds to nearest (+0.5
before int()) to match mfb, whose naive digit-accumulation toFloat is not
correctly rounded (e.g. "2.375000" -> one ULP low); the parse error is far under
0.5 micro-units, so rounding recovers the exact i*125000 in every language.
"""
import sys

RUN = 1
now_ns = None
record = None

CONV_N = 100000


def test_convert_int():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(CONV_N):
            s = str(i)          # toString(i)
            back = int(s)       # toInt(s)
            acc += back + len(s)
        checksum = acc
        times.append(now_ns() - t0)
    print("convert_int = %d" % checksum, file=sys.stderr)
    record("convert", "int", times)


def test_convert_float():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(CONV_N):
            v = i / 8.0                 # exact dyadic rational
            s = "%.6f" % v              # toString(v, 6)
            b = float(s)                # toFloat(s)
            acc += int(b * 1000000.0 + 0.5) + len(s)  # round -> i*125000 + len
        checksum = acc
        times.append(now_ns() - t0)
    print("convert_float = %d" % checksum, file=sys.stderr)
    record("convert", "float", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_convert_int()
    test_convert_float()
