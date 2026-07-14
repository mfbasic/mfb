"""String-builder benchmarks + the realistic-size unicode churn row.

Mirrors benchmark/mfb/src/strbuild.mfb (group `strbuild`: concat/join/splitjoin/
clean) and the `string unibig` row from benchmark/mfb/src/string.mfb.

The strbuild rows are ASCII so their checksums line up with mfb exactly. The
`unibig` row's grapheme segmentation is approximate (see stringbench.py's
`unicode` row) so exact cross-language parity is NOT expected there -- only a
stable, reproducible number. The tiny loop counts match mfb so the table aligns.
"""
import sys
import unicodedata

try:
    import regex as _regex   # optional; only used for proper \X grapheme clusters
except ImportError:
    _regex = None

RUN = 1
now_ns = None
record = None


def _graphemes(s):
    if _regex is not None:
        return _regex.findall(r"\X", s)
    clusters = []
    for ch in s:
        if clusters and 0x0300 <= ord(ch) <= 0x036F:
            clusters[-1] += ch
        else:
            clusters.append(ch)
    return clusters


# Unicode at realistic size -- graphemes / graphemeAt / normalizeNfc / caseFold
# over a multi-KB mixed-script string. Approximate checksum (grapheme surface),
# NOT expected to match mfb; stable and reproducible. Group `string`.
def test_string_unibig():
    # frag == mfb's "caf\u{00E9} \u{4E2D}\u{6587} rocket\u{1F680} nai\u{0308}ve
    # Stra\u{00DF}e " -- 30 code points, decomposed "naïve" (i + U+0308).
    frag = "caf\u00e9 \u4e2d\u6587 rocket\U0001f680 nai\u0308ve Stra\u00dfe "
    base = frag * 8
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _i in range(5):
            g = _graphemes(base)
            acc += len(g)
            acc += len(g)                                     # graphemesCount
            acc += len(g[0]) if g else 0                      # graphemeAt(base, 0)
            acc += len(unicodedata.normalize("NFC", base))
            acc += len(base.casefold())
        checksum = acc
        times.append(now_ns() - t0)
    print("string_unibig = %d" % checksum, file=sys.stderr)
    record("string", "unibig", times)


def test_strbuild_concat():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        s = ""
        for i in range(4000):
            s = s + str(i) + ","
        checksum = len(s)
        times.append(now_ns() - t0)
    print("strbuild_concat = %d" % checksum, file=sys.stderr)
    record("strbuild", "concat", times)


def test_strbuild_join():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        parts = []
        for i in range(4000):
            parts.append(str(i))
        s = ",".join(parts) + ","
        checksum = len(s)
        times.append(now_ns() - t0)
    print("strbuild_join = %d" % checksum, file=sys.stderr)
    record("strbuild", "join", times)


def test_strbuild_splitjoin():
    fields = ["field" + str(i) for i in range(100)]
    line = ",".join(fields)
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _p in range(2000):
            parts = line.split(",")
            rejoined = ",".join(parts)
            acc += len(parts) + len(rejoined)
        checksum = acc
        times.append(now_ns() - t0)
    print("strbuild_splitjoin = %d" % checksum, file=sys.stderr)
    record("strbuild", "splitjoin", times)


def test_strbuild_clean():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(20000):
            s = "  key_" + str(i) + "__  "
            t = s.strip(" _")
            t = t.replace("key", "K")
            if t.startswith("K"):
                t = t[1:]
            if len(t) < 12:
                t = " " * (12 - len(t)) + t
            acc += len(t)
        checksum = acc
        times.append(now_ns() - t0)
    print("strbuild_clean = %d" % checksum, file=sys.stderr)
    record("strbuild", "clean", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_string_unibig()
    test_strbuild_concat()
    test_strbuild_join()
    test_strbuild_splitjoin()
    test_strbuild_clean()
