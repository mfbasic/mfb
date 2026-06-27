"""Classic 5-body simulation (Sun, Jupiter, Saturn, Uranus, Neptune) from the
Computer Language Benchmarks Game. advance(dt=0.01) for STEPS steps; prints the
total energy before and after to 9 decimals.
"""

import math

PI = 3.141592653589793
SOLAR_MASS = 4.0 * PI * PI
DAYS_PER_YEAR = 365.24
NBODIES = 5
STEPS = 100000

x = [0.0] * NBODIES
y = [0.0] * NBODIES
z = [0.0] * NBODIES
vx = [0.0] * NBODIES
vy = [0.0] * NBODIES
vz = [0.0] * NBODIES
mass = [0.0] * NBODIES


def setup():
    # Sun
    x[0] = 0.0; y[0] = 0.0; z[0] = 0.0
    vx[0] = 0.0; vy[0] = 0.0; vz[0] = 0.0
    mass[0] = SOLAR_MASS

    # Jupiter
    x[1] = 4.84143144246472090e+00
    y[1] = -1.16032004402742839e+00
    z[1] = -1.03622044471123109e-01
    vx[1] = 1.66007664274403694e-03 * DAYS_PER_YEAR
    vy[1] = 7.69901118419740425e-03 * DAYS_PER_YEAR
    vz[1] = -6.90460016972063023e-05 * DAYS_PER_YEAR
    mass[1] = 9.54791938424326609e-04 * SOLAR_MASS

    # Saturn
    x[2] = 8.34336671824457987e+00
    y[2] = 4.12479856412430479e+00
    z[2] = -4.03523417114321381e-01
    vx[2] = -2.76742510726862411e-03 * DAYS_PER_YEAR
    vy[2] = 4.99852801234917238e-03 * DAYS_PER_YEAR
    vz[2] = 2.30417297573763929e-05 * DAYS_PER_YEAR
    mass[2] = 2.85885980666130812e-04 * SOLAR_MASS

    # Uranus
    x[3] = 1.28943695621391310e+01
    y[3] = -1.51111514016986312e+01
    z[3] = -2.23307578892655734e-01
    vx[3] = 2.96460137564761618e-03 * DAYS_PER_YEAR
    vy[3] = 2.37847173959480950e-03 * DAYS_PER_YEAR
    vz[3] = -2.96589568540237556e-05 * DAYS_PER_YEAR
    mass[3] = 4.36624404335156298e-05 * SOLAR_MASS

    # Neptune
    x[4] = 1.53796971148509165e+01
    y[4] = -2.59193146099879641e+01
    z[4] = 1.79258772950371181e-01
    vx[4] = 2.68067772490389322e-03 * DAYS_PER_YEAR
    vy[4] = 1.62824170038242295e-03 * DAYS_PER_YEAR
    vz[4] = -9.51592254519715870e-05 * DAYS_PER_YEAR
    mass[4] = 5.15138902046611451e-05 * SOLAR_MASS


def offset_momentum():
    px = 0.0
    py = 0.0
    pz = 0.0
    for i in range(NBODIES):
        px = px + vx[i] * mass[i]
        py = py + vy[i] * mass[i]
        pz = pz + vz[i] * mass[i]
    vx[0] = -px / SOLAR_MASS
    vy[0] = -py / SOLAR_MASS
    vz[0] = -pz / SOLAR_MASS


def advance(dt):
    for i in range(NBODIES):
        for j in range(i + 1, NBODIES):
            dx = x[i] - x[j]
            dy = y[i] - y[j]
            dz = z[i] - z[j]
            d2 = dx * dx + dy * dy + dz * dz
            dist = math.sqrt(d2)
            mag = dt / (d2 * dist)
            vx[i] = vx[i] - dx * mass[j] * mag
            vy[i] = vy[i] - dy * mass[j] * mag
            vz[i] = vz[i] - dz * mass[j] * mag
            vx[j] = vx[j] + dx * mass[i] * mag
            vy[j] = vy[j] + dy * mass[i] * mag
            vz[j] = vz[j] + dz * mass[i] * mag
    for i in range(NBODIES):
        x[i] = x[i] + dt * vx[i]
        y[i] = y[i] + dt * vy[i]
        z[i] = z[i] + dt * vz[i]


def energy():
    e = 0.0
    for i in range(NBODIES):
        e = e + 0.5 * mass[i] * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i])
        for j in range(i + 1, NBODIES):
            dx = x[i] - x[j]
            dy = y[i] - y[j]
            dz = z[i] - z[j]
            dist = math.sqrt(dx * dx + dy * dy + dz * dz)
            e = e - mass[i] * mass[j] / dist
    return e


def main():
    setup()
    offset_momentum()
    print("energy: %.9f" % energy())
    for _ in range(STEPS):
        advance(0.01)
    print("energy: %.9f" % energy())
    return 0


if __name__ == "__main__":
    main()
