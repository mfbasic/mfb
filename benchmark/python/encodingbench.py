"""Encoding-group benchmark (encoding:: package surface).

Mirrors benchmark/mfb/src/encoding.mfb: base64/hex/percent encode-decode round
trips over a deterministic byte buffer (base64/hex) or a URL-ish string
(percent). Python's base64/bytes.hex/urllib.parse produce RFC 4648 base64/hex and
RFC 3986 percent-encoding (safe=""), so the checksums line up with the mfb and C
references bit-for-bit. Each checksum folds the decoded bytes (== the input) plus
the encoded length.
"""
import base64
import sys
import urllib.parse

RUN = 1
now_ns = None
record = None


def _buf(n):
    return bytes((i * 37 + 11) % 256 for i in range(n))


def test_encoding_base64():
    # Arena-gated in mfb (plan-44-J): arena quadratic fixed by plan-64-A1; buffer/reps at target.
    buf = _buf(4096)
    reps = 200
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(reps):
            enc = base64.b64encode(buf).decode("ascii")
            dec = base64.b64decode(enc)
            acc += sum(dec) + len(enc)
        checksum = acc
        times.append(now_ns() - t0)
    print("encoding_base64 = %d" % checksum, file=sys.stderr)
    record("encoding", "base64", times)


def test_encoding_hex():
    buf = _buf(512)
    reps = 8
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(reps):
            enc = buf.hex()
            dec = bytes.fromhex(enc)
            acc += sum(dec) + len(enc)
        checksum = acc
        times.append(now_ns() - t0)
    print("encoding_hex = %d" % checksum, file=sys.stderr)
    record("encoding", "hex", times)


def test_encoding_percent():
    src = "https://example.com/search?q=hello world&lang=en_US#section-1 (v2.0) 100% done"
    reps = 16
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(reps):
            enc = urllib.parse.quote(src, safe="")
            dec = urllib.parse.unquote(enc)
            acc += sum(dec.encode("utf-8")) + len(enc)
        checksum = acc
        times.append(now_ns() - t0)
    print("encoding_percent = %d" % checksum, file=sys.stderr)
    record("encoding", "percent", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_encoding_base64()
    test_encoding_hex()
    test_encoding_percent()
