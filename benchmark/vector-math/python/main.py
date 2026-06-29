#!/usr/bin/env python3
"""3D vector-math throughput benchmark, mirroring benchmark/vector-math/mfb.

Each of 200,000 iterations builds two index-derived 3-vectors and runs the same
sequence of operations the vector:: package performs -- normalize, cross, lerp,
scale, dot, length, distance -- folding the results into one accumulator. The
operation order matches the MFBASIC and C references so all three print the same
value.
"""
from math import sqrt


def main() -> None:
    acc = 0.0
    for k in range(200000):
        fk = float(k)
        ax, ay, az = fk + 1.0, fk * 0.5 + 2.0, 3.0 - fk * 0.25
        bx, by, bz = 2.0 - fk * 0.125, fk + 0.5, fk * 0.75 + 1.0

        # normalize(a), normalize(b)
        la = sqrt(ax * ax + ay * ay + az * az)
        nax, nay, naz = ax / la, ay / la, az / la
        lb = sqrt(bx * bx + by * by + bz * bz)
        nbx, nby, nbz = bx / lb, by / lb, bz / lb

        # cross(na, nb)
        cx = nay * nbz - naz * nby
        cy = naz * nbx - nax * nbz
        cz = nax * nby - nay * nbx

        # lerp(a, b, 0.5)
        tc = 0.5
        mx = ax + (bx - ax) * tc
        my = ay + (by - ay) * tc
        mz = az + (bz - az) * tc

        # scale(na, nb) -- component-wise (Hadamard) product
        sx, sy, sz = nax * nbx, nay * nby, naz * nbz

        # dot(c, m)
        dcm = cx * mx + cy * my + cz * mz
        # length(s)
        lens = sqrt(sx * sx + sy * sy + sz * sz)
        # distance(a, b)
        dx, dy, dz = ax - bx, ay - by, az - bz
        dist = sqrt(dx * dx + dy * dy + dz * dz)

        acc = acc + dcm + lens + dist
    print(f"acc: {acc:.6f}")


if __name__ == "__main__":
    main()
