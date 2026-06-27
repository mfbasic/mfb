"""ulp.py — IEEE-754 double helpers shared by the math-kernel tooling.

Small, dependency-free (stdlib only). Used by gen_coeffs.py to report the f64
accuracy of generated polynomials, and handy when eyeballing reference vectors.
"""
import math
import struct


def f64_to_bits(x):
    return struct.unpack(">Q", struct.pack(">d", float(x)))[0]


def bits_to_f64(b):
    return struct.unpack(">d", struct.pack(">Q", b & 0xFFFFFFFFFFFFFFFF))[0]


def hex_bits(x):
    """16-hex-digit IEEE-754 pattern (matches capture_ref's %016llx)."""
    return struct.pack(">d", float(x)).hex()


def ulp_diff(a, b):
    """Signed-magnitude ULP distance between two finite doubles.

    Uses the standard monotone ordinal mapping so that adjacent doubles differ
    by exactly 1, correctly spanning the +0/-0 boundary. Returns a (possibly
    huge) integer; NaN/inf inputs raise.
    """
    a = float(a)
    b = float(b)
    if not (math.isfinite(a) and math.isfinite(b)):
        raise ValueError("ulp_diff requires finite inputs")

    def ordinal(x):
        bits = f64_to_bits(x)
        # Map so that the doubles are a contiguous monotone integer sequence.
        if bits & 0x8000000000000000:
            return 0x8000000000000000 - bits
        return bits | 0x8000000000000000

    return abs(ordinal(a) - ordinal(b))
