"""Record-shaped workloads the way each language writes them (P1 coverage).

Mirrors benchmark/mfb/src/recordbench.mfb and benchmark/c/recordbench.c:

  record wide / nested -- a 10-field mixed int/float/str dataclass updated with
    dataclasses.replace (a copy, like mfb's WITH) through a function in a hot
    loop; `nested` holds it inside another dataclass inside a list (read element,
    replace, write back).
  float nbody_rec / nbody_vec -- the SAME simulation as `float nbody` in main.py
    (same bodies, 100000 steps, same pair and float op order) over a list of
    Body objects, and over bodies with (x, y, z) tuple position/velocity and
    dot/distance helpers.

Checksums match mfb/C: every record-row float op is an add or a multiply by a
power of two (exact), and the n-body rows fold round-half-away(energy * 1e9) --
the nine decimals `float nbody` prints.
"""
import math
import sys
from dataclasses import dataclass, replace

RUN = 1
now_ns = None
record = None


# ===================================================================== #
# GROUP: record (wide / nested)                                         #
# ===================================================================== #

@dataclass
class Wide:
    id: int
    count: int
    total: int
    flags: int
    pos: float
    vel: float
    energy: float
    name: str
    tag: str
    weight: float


@dataclass
class Holder:
    id: int
    label: str
    inner: Wide
    touches: int


def step_wide(w, i, tag):
    return replace(w, count=w.count + 1, total=w.total + i % 7,
                   flags=(w.flags * 5 + i) % 1024, pos=w.pos + w.vel * 0.25,
                   vel=w.vel * 0.5 + float(i % 4), energy=w.energy + w.vel, tag=tag)


def wide_sum(w):
    return (w.id + w.count + w.total + w.flags + math.floor(w.pos * 4.0)
            + math.floor(w.vel * 1024.0) + math.floor(w.energy) + len(w.name)
            + len(w.tag) + math.floor(w.weight * 8.0))


def new_wide(id_):
    return Wide(id_, 0, id_, id_ % 1024, id_ * 0.5, 1.0, 0.0, "wide-%d" % id_, "", id_ * 0.125)


def test_record_wide():
    times = []
    checksum = 0
    for _ in range(RUN):
        w = new_wide(7)
        t0 = now_ns()
        for i in range(1000000):
            if i % 2 == 0:
                w = step_wide(w, i, "even-step")
            else:
                w = step_wide(w, i, "odd")
        times.append(now_ns() - t0)
        checksum = wide_sum(w)
    print("test_record_wide = %d" % checksum, file=sys.stderr)
    record("record", "wide", times)


def test_record_nested():
    times = []
    checksum = 0
    for _ in range(RUN):
        holders = [Holder(k, "holder-%d" % k, new_wide(k), 0) for k in range(64)]
        t0 = now_ns()
        for i in range(300000):
            k = i % 64
            h = holders[k]
            tag = "even-step" if i % 2 == 0 else "odd"
            holders[k] = replace(h, inner=step_wide(h.inner, i, tag), touches=h.touches + 1)
        times.append(now_ns() - t0)
        checksum = sum(h.id + len(h.label) + h.touches + wide_sum(h.inner) for h in holders)
    print("test_record_nested = %d" % checksum, file=sys.stderr)
    record("record", "nested", times)


# ===================================================================== #
# GROUP: float (nbody_rec / nbody_vec)                                  #
# ===================================================================== #

class Body:
    __slots__ = ("x", "y", "z", "vx", "vy", "vz", "mass")

    def __init__(self, x, y, z, vx, vy, vz, mass):
        self.x, self.y, self.z = x, y, z
        self.vx, self.vy, self.vz = vx, vy, vz
        self.mass = mass


def nbody_bodies():
    PI = 3.141592653589793
    SOLAR_MASS = 4.0 * PI * PI
    DPY = 365.24
    bodies = [
        Body(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, SOLAR_MASS),
        Body(4.84143144246472090, -1.16032004402742839, -1.03622044471123109e-01,
             1.66007664274403694e-03 * DPY, 7.69901118419740425e-03 * DPY, -6.90460016972063023e-05 * DPY,
             9.54791938424326609e-04 * SOLAR_MASS),
        Body(8.34336671824457987, 4.12479856412430479, -4.03523417114321381e-01,
             -2.76742510726862411e-03 * DPY, 4.99852801234917238e-03 * DPY, 2.30417297573763929e-05 * DPY,
             2.85885980666130812e-04 * SOLAR_MASS),
        Body(1.28943695621391310e+01, -1.51111514016986312e+01, -2.23307578892655734e-01,
             2.96460137564761618e-03 * DPY, 2.37847173959480950e-03 * DPY, -2.96589568540237556e-05 * DPY,
             4.36624404335156298e-05 * SOLAR_MASS),
        Body(1.53796971148509165e+01, -2.59193146099879641e+01, 1.79258772950371181e-01,
             2.68067772490389322e-03 * DPY, 1.62824170038242295e-03 * DPY, -9.51592254519715870e-05 * DPY,
             5.15138902046611451e-05 * SOLAR_MASS),
    ]
    px = py = pz = 0.0
    for b in bodies:
        px += b.vx * b.mass
        py += b.vy * b.mass
        pz += b.vz * b.mass
    bodies[0].vx = -px / SOLAR_MASS
    bodies[0].vy = -py / SOLAR_MASS
    bodies[0].vz = -pz / SOLAR_MASS
    return bodies


def round_half_away(v):
    # mfb math::round / C llround: nearest, halves away from zero.
    return math.floor(v + 0.5) if v >= 0 else -math.floor(-v + 0.5)


def test_nbody_rec():
    dt = 0.01
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        bodies = nbody_bodies()
        n = len(bodies)
        for _s in range(100000):
            for i in range(n):
                bi = bodies[i]
                for j in range(i + 1, n):
                    bj = bodies[j]
                    dx = bi.x - bj.x
                    dy = bi.y - bj.y
                    dz = bi.z - bj.z
                    d2 = dx * dx + dy * dy + dz * dz
                    mag = dt / (d2 * math.sqrt(d2))
                    bi.vx -= dx * bj.mass * mag
                    bi.vy -= dy * bj.mass * mag
                    bi.vz -= dz * bj.mass * mag
                    bj.vx += dx * bi.mass * mag
                    bj.vy += dy * bi.mass * mag
                    bj.vz += dz * bi.mass * mag
            for b in bodies:
                b.x += dt * b.vx
                b.y += dt * b.vy
                b.z += dt * b.vz
        e = 0.0
        for i in range(n):
            bi = bodies[i]
            e += 0.5 * bi.mass * (bi.vx * bi.vx + bi.vy * bi.vy + bi.vz * bi.vz)
            for j in range(i + 1, n):
                bj = bodies[j]
                dx = bi.x - bj.x
                dy = bi.y - bj.y
                dz = bi.z - bj.z
                e -= bi.mass * bj.mass / math.sqrt(dx * dx + dy * dy + dz * dz)
        checksum = round_half_away(e * 1000000000.0)
        times.append(now_ns() - t0)
    print("test_float_nbody_rec = %d" % checksum, file=sys.stderr)
    record("float", "nbody_rec", times)


class VBody:
    __slots__ = ("pos", "vel", "mass")

    def __init__(self, pos, vel, mass):
        self.pos, self.vel, self.mass = pos, vel, mass


def vdot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def vdistance(a, b):
    d = (a[0] - b[0], a[1] - b[1], a[2] - b[2])
    return math.sqrt(vdot(d, d))


def test_nbody_vec():
    dt = 0.01
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        bodies = [VBody((b.x, b.y, b.z), (b.vx, b.vy, b.vz), b.mass) for b in nbody_bodies()]
        n = len(bodies)
        for _s in range(100000):
            for i in range(n):
                bi = bodies[i]
                for j in range(i + 1, n):
                    bj = bodies[j]
                    pi, pj = bi.pos, bj.pos
                    d = (pi[0] - pj[0], pi[1] - pj[1], pi[2] - pj[2])
                    d2 = vdot(d, d)
                    mag = dt / (d2 * math.sqrt(d2))
                    vi, vj = bi.vel, bj.vel
                    bi.vel = (vi[0] - d[0] * bj.mass * mag, vi[1] - d[1] * bj.mass * mag,
                              vi[2] - d[2] * bj.mass * mag)
                    bj.vel = (vj[0] + d[0] * bi.mass * mag, vj[1] + d[1] * bi.mass * mag,
                              vj[2] + d[2] * bi.mass * mag)
            for b in bodies:
                p, v = b.pos, b.vel
                b.pos = (p[0] + dt * v[0], p[1] + dt * v[1], p[2] + dt * v[2])
        e = 0.0
        for i in range(n):
            bi = bodies[i]
            e += 0.5 * bi.mass * vdot(bi.vel, bi.vel)
            for j in range(i + 1, n):
                e -= bi.mass * bodies[j].mass / vdistance(bi.pos, bodies[j].pos)
        checksum = round_half_away(e * 1000000000.0)
        times.append(now_ns() - t0)
    print("test_float_nbody_vec = %d" % checksum, file=sys.stderr)
    record("float", "nbody_vec", times)


def run_float(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_nbody_rec()
    test_nbody_vec()


def run_record(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_record_wide()
    test_record_nested()
