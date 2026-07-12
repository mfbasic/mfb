"""Vector-group benchmarks (vector:: package surface, scalar-equivalent).

Mirrors benchmark/mfb/src/vector.mfb:
  - `math`  : the historical Float3 geometry row (moved out of main.py).
  - `float` : test_vector_float -- every vector:: member on Float2, plus the
              3D/4D cross overloads.
  - `int`   : test_vector_int  -- the same surface on integer components, with
              each accumulated term rounded half-away-from-zero.

Fixed (test_vector_fixed) is omitted: Fixed is an mfb-only type. Every vector
member is implemented inline as plain scalar component math; exact cross-language
parity is not required for these rows -- they are matched-loop-bound workloads.
"""
import math
import sys
from math import acos, cos, sin, sqrt

RUN = 1
now_ns = None
record = None


def _round_half_away(x):
    if x >= 0.0:
        return math.floor(x + 0.5)
    return math.ceil(x - 0.5)


def _clamp(v, lo, hi):
    if v < lo:
        return lo
    if v > hi:
        return hi
    return v


# --- generic component-vector helpers (lists of any dimension) --------------

def vadd(a, b):
    return [x + y for x, y in zip(a, b)]


def vsub(a, b):
    return [x - y for x, y in zip(a, b)]


def vscl(a, s):
    return [x * s for x in a]


def vhad(a, b):                       # scale(a, b): component-wise product
    return [x * y for x, y in zip(a, b)]


def vdot(a, b):
    return sum(x * y for x, y in zip(a, b))


def vlen(a):
    return sqrt(vdot(a, a))


def vdist(a, b):
    return vlen(vsub(a, b))


def vabs(a):
    return [abs(x) for x in a]


def vmin(a, b):
    return [min(x, y) for x, y in zip(a, b)]


def vmax(a, b):
    return [max(x, y) for x, y in zip(a, b)]


def vnorm(a):
    l = vlen(a)
    return vscl(a, 1.0 / l) if l > 0.0 else list(a)


def vangle(a, b):
    la, lb = vlen(a), vlen(b)
    if la == 0.0 or lb == 0.0:
        return 0.0
    return acos(_clamp(vdot(a, b) / (la * lb), -1.0, 1.0))


def vlerp(a, b, t):
    tc = _clamp(t, 0.0, 1.0)
    return [ax + (bx - ax) * tc for ax, bx in zip(a, b)]


def vlerp_un(a, b, t):
    return [ax + (bx - ax) * t for ax, bx in zip(a, b)]


def vclamp_len(a, maxlen):
    l = vlen(a)
    if l > maxlen and l > 0.0:
        return vscl(a, maxlen / l)
    return list(a)


def vproject(a, b):
    d = vdot(b, b)
    if d == 0.0:
        return [0.0 for _ in a]
    return vscl(b, vdot(a, b) / d)


def vreject(a, b):
    return vsub(a, vproject(a, b))


def vreflect(a, n):
    return vsub(a, vscl(n, 2.0 * vdot(a, n)))


def vslerp(a, b, t):
    # Mirrors the mfb canonical __vector_slerp_* : slerp the RAW vectors
    # (w0*a + w1*b), degenerate fallback = unclamped lerp, threshold 1e-6.
    omega = vangle(a, b)
    s = sin(omega)
    if abs(s) < 1e-6:
        return vlerp_un(a, b, t)
    w0 = sin((1.0 - t) * omega) / s
    w1 = sin(t * omega) / s
    return [w0 * ax + w1 * bx for ax, bx in zip(a, b)]


def vperp2(a):
    return [-a[1], a[0]]


def vrotate2(a, ang):
    c, s = cos(ang), sin(ang)
    return [a[0] * c - a[1] * s, a[0] * s + a[1] * c]


def vcross2(a):                       # 1-ary 2D cross == perpendicular
    return [-a[1], a[0]]


def vcross3(a, b):                    # 2-ary 3D cross
    return [a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0]]


def _det3(m):
    return (m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]))


def vcross4(a, b, c):                 # 3-ary 4D cross (orthogonal to a, b, c)
    res = [0.0, 0.0, 0.0, 0.0]
    for i in range(4):
        cols = [j for j in range(4) if j != i]
        m = [[a[cols[0]], a[cols[1]], a[cols[2]]],
             [b[cols[0]], b[cols[1]], b[cols[2]]],
             [c[cols[0]], c[cols[1]], c[cols[2]]]]
        res[i] = (1.0 if i % 2 == 0 else -1.0) * _det3(m)
    return res


# --- rows -------------------------------------------------------------------

def test_vector_math():
    times = []
    checksum = 0.0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0.0
        for k in range(200000):
            fk = float(k)
            ax, ay, az = fk + 1.0, fk * 0.5 + 2.0, 3.0 - fk * 0.25
            bx, by, bz = 2.0 - fk * 0.125, fk + 0.5, fk * 0.75 + 1.0
            la = sqrt(ax * ax + ay * ay + az * az)
            nax, nay, naz = ax / la, ay / la, az / la
            lb = sqrt(bx * bx + by * by + bz * bz)
            nbx, nby, nbz = bx / lb, by / lb, bz / lb
            cx = nay * nbz - naz * nby
            cy = naz * nbx - nax * nbz
            cz = nax * nby - nay * nbx
            mx = ax + (bx - ax) * 0.5
            my = ay + (by - ay) * 0.5
            mz = az + (bz - az) * 0.5
            sx, sy, sz = nax * nbx, nay * nby, naz * nbz
            dcm = cx * mx + cy * my + cz * mz
            lens = sqrt(sx * sx + sy * sy + sz * sz)
            dx, dy, dz = ax - bx, ay - by, az - bz
            dist = sqrt(dx * dx + dy * dy + dz * dz)
            acc += dcm + lens + dist
        checksum = acc
        times.append(now_ns() - t0)
    print("vector_math = %.6f" % checksum, file=sys.stderr)
    record("vector", "math", times)


# Float family: every vector:: member on Float2 + 3D/4D cross overloads.
def test_vector_float():
    times = []
    checksum = 0.0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0.0
        for i in range(20000):
            fk = float(i - (i // 1000) * 1000)
            a = [fk + 1.0, fk * 0.5 + 2.0]
            b = [fk * 0.25 + 3.0, fk + 1.5]
            nb = vnorm(b)
            acc += vlen(a)
            acc += vdist(a, b)
            acc += vdot(a, b)
            acc += vangle(a, b)
            acc += vlen(vabs(a))
            acc += vlen(vmin(a, b))
            acc += vlen(vmax(a, b))
            acc += vlen(vhad(a, b))
            acc += vlen(vnorm(a))
            acc += vlen(vlerp(a, b, 0.5))
            acc += vlen(vlerp_un(a, b, 1.5))
            acc += vlen(vclamp_len(a, 3.0))
            acc += vlen(vproject(a, b))
            acc += vlen(vreject(a, b))
            acc += vlen(vreflect(a, nb))
            acc += vlen(vslerp(a, b, 0.5))
            acc += vlen(vperp2(a))
            acc += vlen(vrotate2(a, 0.5))
            acc += vlen(vcross2(a))
            a3 = [fk + 1.0, fk * 0.5 + 2.0, fk * 0.25 + 3.0]
            b3 = [fk * 0.3 + 1.0, fk + 2.0, fk * 0.7 + 0.5]
            acc += vlen(vcross3(a3, b3))
            a4 = [fk + 1.0, fk * 0.5 + 2.0, fk * 0.25 + 3.0, fk * 0.1 + 1.0]
            b4 = [fk * 0.3 + 1.0, fk + 2.0, fk * 0.7 + 0.5, fk * 0.2 + 2.0]
            c4 = [fk * 0.6 + 1.0, fk * 0.2 + 2.0, fk + 0.5, fk * 0.9 + 1.0]
            acc += vlen(vcross4(a4, b4, c4))
        checksum = acc
        times.append(now_ns() - t0)
    print("vector_float = %.3f" % checksum, file=sys.stderr)
    record("vector", "float", times)


# Integer family: same surface, each accumulated term rounded half-away.
def test_vector_int():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for i in range(20000):
            m = i - (i // 90) * 90
            a = [m + 1, m + 2]
            b = [m + 3, m + 5]
            nb = vnorm(b)
            acc += _round_half_away(vlen(a))
            acc += _round_half_away(vdist(a, b))
            acc += vdot(a, b)
            acc += _round_half_away(vangle(a, b))
            acc += _round_half_away(vlen(vabs(a)))
            acc += _round_half_away(vlen(vmin(a, b)))
            acc += _round_half_away(vlen(vmax(a, b)))
            acc += _round_half_away(vlen(vhad(a, b)))
            acc += _round_half_away(vlen(vnorm(a)))
            acc += _round_half_away(vlen(vlerp(a, b, 0.5)))
            acc += _round_half_away(vlen(vlerp_un(a, b, 1.5)))
            acc += _round_half_away(vlen(vclamp_len(a, 50)))
            acc += _round_half_away(vlen(vproject(a, b)))
            acc += _round_half_away(vlen(vreject(a, b)))
            acc += _round_half_away(vlen(vreflect(a, nb)))
            acc += _round_half_away(vlen(vslerp(a, b, 0.5)))
            acc += _round_half_away(vlen(vperp2(a)))
            acc += _round_half_away(vlen(vrotate2(a, 0.5)))
            acc += _round_half_away(vlen(vcross2(a)))
            a3 = [m + 1, m + 2, m + 3]
            b3 = [m + 4, m + 5, m + 6]
            acc += _round_half_away(vlen(vcross3(a3, b3)))
            a4 = [m + 1, m + 2, m + 3, m + 4]
            b4 = [m + 2, m + 3, m + 4, m + 5]
            c4 = [m + 3, m + 4, m + 5, m + 6]
            acc += _round_half_away(vlen(vcross4(a4, b4, c4)))
        checksum = acc
        times.append(now_ns() - t0)
    print("vector_int = %d" % checksum, file=sys.stderr)
    record("vector", "int", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_vector_math()
    test_vector_float()
    test_vector_int()
