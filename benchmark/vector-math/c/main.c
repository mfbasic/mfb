/* 3D vector-math throughput benchmark, mirroring benchmark/vector-math/mfb.
 *
 * Each of 200,000 iterations builds two index-derived 3-vectors and runs the
 * same sequence of operations the vector:: package performs -- normalize, cross,
 * lerp, scale, dot, length, distance -- folding the results into one
 * accumulator. The operation order matches the MFBASIC and Python references so
 * all three print the same value. */
#include <stdio.h>
#include <math.h>

int main(void) {
  double acc = 0.0;
  for (long k = 0; k < 200000; k++) {
    double fk = (double)k;
    double ax = fk + 1.0, ay = fk * 0.5 + 2.0, az = 3.0 - fk * 0.25;
    double bx = 2.0 - fk * 0.125, by = fk + 0.5, bz = fk * 0.75 + 1.0;

    /* normalize(a), normalize(b) */
    double la = sqrt(ax * ax + ay * ay + az * az);
    double nax = ax / la, nay = ay / la, naz = az / la;
    double lb = sqrt(bx * bx + by * by + bz * bz);
    double nbx = bx / lb, nby = by / lb, nbz = bz / lb;

    /* cross(na, nb) */
    double cx = nay * nbz - naz * nby;
    double cy = naz * nbx - nax * nbz;
    double cz = nax * nby - nay * nbx;

    /* lerp(a, b, 0.5) */
    double tc = 0.5;
    double mx = ax + (bx - ax) * tc;
    double my = ay + (by - ay) * tc;
    double mz = az + (bz - az) * tc;

    /* scale(na, nb) -- component-wise (Hadamard) product */
    double sx = nax * nbx, sy = nay * nby, sz = naz * nbz;

    /* dot(c, m) */
    double dcm = cx * mx + cy * my + cz * mz;
    /* length(s) */
    double lens = sqrt(sx * sx + sy * sy + sz * sz);
    /* distance(a, b) */
    double dx = ax - bx, dy = ay - by, dz = az - bz;
    double dist = sqrt(dx * dx + dy * dy + dz * dz);

    acc = acc + dcm + lens + dist;
  }
  printf("acc: %.6f\n", acc);
  return 0;
}
