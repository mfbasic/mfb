"""crypto-group benchmark (crypto:: package surface).

Mirrors benchmark/mfb/src/crypto.mfb: SHA-256/512, HMAC-SHA-256, PBKDF2-HMAC-
SHA-256, constant-time compare, a fresh-message hash churn, and Ed25519
sign+verify. Python uses the stdlib hashlib/hmac (standard FIPS/RFC algorithms)
and, for Ed25519, pyca `cryptography`; every core matches the mfb `crypto::`
software core bit-for-bit, so each checksum (a fold of the digest/tag/signature
bytes) lines up with the mfb and C references exactly.

The Ed25519 row has no in-suite C peer (C prints `--`); mfb and Python agree
because RFC-8032 signing is deterministic and the seed is fixed.
"""
import hashlib
import hmac as _hmac
import sys

from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey, Ed25519PublicKey)

RUN = 1
now_ns = None
record = None


def _buf(n):
    return bytes((i * 37 + 11) % 256 for i in range(n))


def test_crypto_sha256():
    buf = _buf(1024)
    reps = 64
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(reps):
            acc += sum(hashlib.sha256(buf).digest())
        checksum = acc
        times.append(now_ns() - t0)
    print("crypto_sha256 = %d" % checksum, file=sys.stderr)
    record("crypto", "sha256", times)


def test_crypto_sha512():
    # Arena-gated in mfb (plan-64-A): SHA-512's >2KB transients hit the O(n^2)
    # free-list walk. The C/Python mirrors keep the same tiny reps to line up.
    buf = _buf(1024)
    reps = 64
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(reps):
            acc += sum(hashlib.sha512(buf).digest())
        checksum = acc
        times.append(now_ns() - t0)
    print("crypto_sha512 = %d" % checksum, file=sys.stderr)
    record("crypto", "sha512", times)


def test_crypto_hmac():
    # Arena-gated in mfb (plan-64-A): per-MAC transient volume drives a cumulative
    # quick-bin climb. Mirrors keep the same tiny reps to line up.
    buf = _buf(1024)
    key = _buf(32)
    reps = 64
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(reps):
            acc += sum(_hmac.new(key, buf, hashlib.sha256).digest())
        checksum = acc
        times.append(now_ns() - t0)
    print("crypto_hmac = %d" % checksum, file=sys.stderr)
    record("crypto", "hmac", times)


def test_crypto_pbkdf2():
    # Arena-gated in mfb (plan-64-A): a 4096-iteration derive is ~8192 SHA-256 ops
    # whose transient churn triggers the flush amplifier. Mirrors keep the low
    # work factor to line up.
    pw = _buf(16)
    salt = _buf(16)
    iters = 4096
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        dk = hashlib.pbkdf2_hmac("sha256", pw, salt, iters, 32)
        checksum = sum(dk)
        times.append(now_ns() - t0)
    print("crypto_pbkdf2 = %d" % checksum, file=sys.stderr)
    record("crypto", "pbkdf2", times)


def test_crypto_cte():
    a = _buf(32)
    b = _buf(32)
    reps = 8192
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        cnt = 0
        for _rep in range(reps):
            if _hmac.compare_digest(a, b):
                cnt += 1
        checksum = cnt
        times.append(now_ns() - t0)
    print("crypto_cte = %d" % checksum, file=sys.stderr)
    record("crypto", "cte", times)


def test_crypto_churn():
    # Arena-gated in mfb (plan-64-A): tiny message count; the C/Python mirrors keep
    # the same count so the table lines up.
    msgs = 4096
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(msgs):
            m = bytes((i * 13 + j * 37 + 11) % 256 for j in range(40))
            acc += sum(hashlib.sha256(m).digest())
        checksum = acc
        times.append(now_ns() - t0)
    print("crypto_churn = %d" % checksum, file=sys.stderr)
    record("crypto", "churn", times)


# Fixed 32-byte Ed25519 seed (b[i] = (i*37+11) mod 256) and its precomputed public
# key, matching benchmark/mfb/src/crypto.mfb. RFC-8032 signing is deterministic, so
# mfb and Python produce the same 64-byte signature and thus the same checksum.
_ED_SEED = _buf(32)
_ED_MSG = _buf(64)
_ED_PUB = bytes([36, 195, 164, 148, 250, 34, 153, 98, 102, 68, 185, 101, 169, 19,
                 45, 167, 156, 254, 103, 21, 31, 220, 66, 222, 96, 12, 240, 144,
                 151, 219, 130, 71])


def test_crypto_ed25519():
    # Arena-gated in mfb (plan-64-A): SHA-512 + Curve25519 field arithmetic over
    # the bits package is transient-allocation heavy. Mirror keeps reps=1.
    sk = Ed25519PrivateKey.from_private_bytes(_ED_SEED)
    pub = Ed25519PublicKey.from_public_bytes(_ED_PUB)
    reps = 4
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for _rep in range(reps):
            sig = sk.sign(_ED_MSG)
            acc += sum(sig)
            try:
                pub.verify(sig, _ED_MSG)
                acc += 1
            except Exception:
                pass
        checksum = acc
        times.append(now_ns() - t0)
    print("crypto_ed25519 = %d" % checksum, file=sys.stderr)
    record("crypto", "ed25519", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_crypto_sha256()
    test_crypto_sha512()
    test_crypto_hmac()
    test_crypto_pbkdf2()
    test_crypto_cte()
    test_crypto_churn()
    test_crypto_ed25519()
