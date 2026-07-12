"""String-group benchmarks (strings:: package surface + & concat).

Mirrors benchmark/mfb/src/string.mfb: the historical `concat` row moved out of
main.py, plus consolidated coverage rows -- case (case/trim/normalize), search
(tests + search), slice (slice/reshape) and unicode (grapheme/byte views).

mfb len() counts Unicode scalars; Python len(str) counts code points -- identical
for the ASCII strings used by case/search/slice, so those checksums line up. The
unicode row's grapheme segmentation is approximate (see comment), so exact
cross-language parity is NOT expected there -- only a stable, reproducible number.
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


def test_string_concat():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        s = ""
        for _i in range(1000):
            s = s + "x"
        checksum = len(s)
        times.append(now_ns() - t0)
    print("string_concat = %d" % checksum, file=sys.stderr)
    record("string", "concat", times)


# Case mapping, trimming, and normalization.
def test_string_case():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(50000):
            s = "  Hello World " + str(i) + "  "
            acc += len(s.upper())
            acc += len(s.lower())
            acc += len(s.casefold())
            acc += len(s.strip())
            acc += len(s.lstrip())
            acc += len(s.rstrip())
            acc += len(s.strip(" Helo"))
            acc += len(unicodedata.normalize("NFC", s))
        checksum = acc
        times.append(now_ns() - t0)
    print("string_case = %d" % checksum, file=sys.stderr)
    record("string", "case", times)


def _starts_with_any(s, prefixes):
    return any(s.startswith(p) for p in prefixes)


def _ends_with_any(s, suffixes):
    return any(s.endswith(x) for x in suffixes)


def _strip_prefix(s, p):
    return s[len(p):] if s.startswith(p) else s


def _strip_suffix(s, p):
    return s[:len(s) - len(p)] if p and s.endswith(p) else s


# Tests and search.
def test_string_search():
    prefixes = ["He", "Wo"]
    suffixes = ["ld", "xx"]
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(50000):
            s = "Hello World " + str(i)
            if "World" in s:
                acc += 1
            acc += s.count("l")
            if s.startswith("Hello"):
                acc += 1
            if s.endswith("World " + str(i)):
                acc += 1
            if _starts_with_any(s, prefixes):
                acc += 1
            if _ends_with_any(s, suffixes):
                acc += 1
            acc += s.find("World")
            acc += len(_strip_prefix(s, "Hello "))
            acc += len(_strip_suffix(s, "!"))
        checksum = acc
        times.append(now_ns() - t0)
    print("string_search = %d" % checksum, file=sys.stderr)
    record("string", "search", times)


def _pad_left(s, width, fill=" "):
    return s if len(s) >= width else fill * (width - len(s)) + s


def _pad_right(s, width, fill=" "):
    return s if len(s) >= width else s + fill * (width - len(s))


# Slicing and reshaping.
def test_string_slice():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(50000):
            s = "Hello World " + str(i)
            acc += len(s[:5])                # left(s, 5)
            acc += len(s[-3:])               # right(s, 3)
            acc += len(s[2:2 + 4])           # mid(s, 2, 4)
            words = s.split(" ")
            acc += len(words)
            acc += len("-".join(words))
            acc += len(s.replace("l", "L"))
            acc += len("ab" * 3)             # repeat("ab", 3)
            acc += len(_pad_left(s, 24))
            acc += len(_pad_right(s, 24, "."))
        checksum = acc
        times.append(now_ns() - t0)
    print("string_slice = %d" % checksum, file=sys.stderr)
    record("string", "slice", times)


def _graphemes(s):
    # Prefer the `regex` module's proper \X extended grapheme clusters when it is
    # importable; otherwise approximate a cluster as a base scalar plus any
    # trailing combining marks in U+0300..U+036F. This approximation does NOT
    # cover ZWJ/emoji-modifier clusters, so the unicode checksum is intentionally
    # not expected to match the mfb reference exactly.
    if _regex is not None:
        return _regex.findall(r"\X", s)
    clusters = []
    for ch in s:
        if clusters and 0x0300 <= ord(ch) <= 0x036F:
            clusters[-1] += ch
        else:
            clusters.append(ch)
    return clusters


# Unicode: grapheme segmentation and byte views.
def test_string_unicode():
    # Decomposed to match mfb "cafe\\u{0301} rocket\\u{1F680} nai\\u{0308}ve schoen":
    # base scalar + combining mark so the grapheme approximation has work to do.
    u = "cafe\u0301 rocket\U0001F680 nai\u0308ve schoen"
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        # Inner count matches the mfb coverage row (10). mfb's grapheme surface
        # triggers an arena mixed-transient-churn slowdown at high counts, so
        # that row is a small coverage smoke-test, not a throughput benchmark;
        # keep the C/Python rows at the same tiny count for a like-for-like table.
        for _i in range(10):
            g = _graphemes(u)
            acc += len(g)
            acc += len(g)                        # graphemesCount(u)
            acc += len(g[0]) if g else 0         # graphemeAt(u, 0)
            byte_len = len(u.encode("utf-8"))
            acc += byte_len                      # byteLen(u)
            acc += byte_len                      # len(toBytes(u))
            acc += len(unicodedata.normalize("NFC", u))
        checksum = acc
        times.append(now_ns() - t0)
    print("string_unicode = %d" % checksum, file=sys.stderr)
    record("string", "unicode", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_string_concat()
    test_string_case()
    test_string_search()
    test_string_slice()
    test_string_unicode()
