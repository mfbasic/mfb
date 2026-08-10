"""Unicode display-width benchmarks (group `width`).

Mirrors benchmark/mfb/src/width.mfb, the first benchmark of
strings::displayWidth (plan-87 Theme 1). Python has no display-width builtin, so
dw_scan hand-rolls the same width logic mfb applies over this *controlled*
corpus: a per-scalar width table (combining marks U+0300..U+036F and ZWJ U+200D
are 0; East-Asian-Wide/emoji are 2; else 1) plus a one-scalar lookahead that
collapses a ZWJ sequence (ZWJ and the scalar after it count 0) and a combining
mark into their lead cluster. Over this corpus that reproduces mfb's
displayWidth/graphemesCount exactly.

The combining mark in the mixed corpus is unambiguously decomposed as the two
UTF-8 bytes for "e" + U+0301 (not a precomposed "e-acute").

Expected checksums: ascii=8250000, mixed=480320, churn=360.
"""
import sys

RUN = 1
now_ns = None
record = None


def _is_zero_width(cp):
    return cp == 0x200D or 0x0300 <= cp <= 0x036F


def _is_wide(cp):
    return (0x4E00 <= cp <= 0x9FFF        # CJK unified ideographs
            or 0x1F300 <= cp <= 0x1FAFF   # emoji / pictographs
            or 0x2600 <= cp <= 0x27BF)    # misc symbols / dingbats


def dw_scan(s):
    """Return (display columns, grapheme clusters) for s over this corpus."""
    cols = clusters = 0
    prev_zwj = False
    for ch in s:
        cp = ord(ch)
        if _is_zero_width(cp):
            prev_zwj = (cp == 0x200D)
            continue
        if prev_zwj:            # ZWJ-sequence continuation
            prev_zwj = False
            continue
        clusters += 1
        cols += 2 if _is_wide(cp) else 1
        prev_zwj = False
    return cols, clusters


def display_width(s):
    return dw_scan(s)[0]


def test_width_ascii():
    frag = "The quick brown fox jumps over the lazy dog 0123456789 "
    base = frag * 75
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        cols = 0
        for _rep in range(2000):
            cols += display_width(base)
        checksum = cols
        times.append(now_ns() - t0)
    print("width_ascii = %d" % checksum, file=sys.stderr)
    record("width", "ascii", times)


def test_width_mixed():
    # abc | U+65E5 U+672C U+8A9E (CJK) | e + U+0301 (combining) |
    # U+1F468 ZWJ U+1F469 ZWJ U+1F467 (man ZWJ woman ZWJ girl)
    frag = ("abc"
            "日本語"
            "é"
            "\U0001f468‍\U0001f469‍\U0001f467")
    base = frag * 40
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        cols = 0
        for _rep in range(1000):
            cols += display_width(base)
        clusters = dw_scan(base)[1]
        checksum = cols + clusters
        times.append(now_ns() - t0)
    print("width_mixed = %d" % checksum, file=sys.stderr)
    record("width", "mixed", times)


def test_width_churn():
    cjk = "日本"  # U+65E5 U+672C — two wide ideographs
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _pass in range(5):
            for i in range(8):
                row = "row%d %s" % (i, cjk)
                acc += display_width(row)
        checksum = acc
        times.append(now_ns() - t0)
    print("width_churn = %d" % checksum, file=sys.stderr)
    record("width", "churn", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_width_ascii()
    test_width_mixed()
    test_width_churn()
