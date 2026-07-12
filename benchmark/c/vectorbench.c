/* GROUP: vector (vector:: package coverage)
 *
 * Moved into its own translation unit; shared timing/recording infra comes
 * from bench.h. Contains the historical `math` row (Float3 geometry) plus the
 * consolidated coverage rows (float, int) that mirror vector.mfb's
 * test_vector_float / test_vector_int as scalar component math. (The Fixed row
 * is mfb-only.)
 *
 * The float/int coverage rows exercise every vector:: member as inline scalar
 * geometry; the mfb driver folds each op's result into an accumulator. Exact
 * cross-language checksum parity is NOT required for these rows (transcendental
 * ops such as angle/slerp diverge per libm, and the Integer row rounds each
 * result) — they are faithful, deterministic workloads with a stable checksum.
 * The Integer row computes each op in double precision and rounds the folded
 * scalar half-away-from-zero, approximating mfb's per-op Integer rounding. */
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "bench.h"
#include "vectorbench.h"

/* ----- historical row: Float3 geometry --------------------------------- */

static void test_vector_math(void) {
  long long *t = alloc_times();
  double checksum = 0.0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    double acc = 0.0;
    for (long k = 0; k < 200000; k++) {
      double fk = (double)k;
      double ax = fk + 1.0, ay = fk * 0.5 + 2.0, az = 3.0 - fk * 0.25;
      double bx = 2.0 - fk * 0.125, by = fk + 0.5, bz = fk * 0.75 + 1.0;
      double la = sqrt(ax * ax + ay * ay + az * az);
      double nax = ax / la, nay = ay / la, naz = az / la;
      double lb = sqrt(bx * bx + by * by + bz * bz);
      double nbx = bx / lb, nby = by / lb, nbz = bz / lb;
      double cx = nay * nbz - naz * nby, cy = naz * nbx - nax * nbz, cz = nax * nby - nay * nbx;
      double mx = ax + (bx - ax) * 0.5, my = ay + (by - ay) * 0.5, mz = az + (bz - az) * 0.5;
      double sx = nax * nbx, sy = nay * nby, sz = naz * nbz;
      double dcm = cx * mx + cy * my + cz * mz;
      double lens = sqrt(sx * sx + sy * sy + sz * sz);
      double dx = ax - bx, dy = ay - by, dz = az - bz;
      double dist = sqrt(dx * dx + dy * dy + dz * dz);
      acc += dcm + lens + dist;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "vector_math = %.6f\n", checksum);
  record("vector", "math", t, RUN);
  free(t);
}

/* ----- 2D / 3D / 4D scalar vector primitives (double math) -------------- */

typedef struct { double x, y; } V2;

static double v2len(V2 a) { return sqrt(a.x * a.x + a.y * a.y); }
static double v2dot(V2 a, V2 b) { return a.x * b.x + a.y * b.y; }
static V2 v2sub(V2 a, V2 b) { return (V2){a.x - b.x, a.y - b.y}; }
static V2 v2add(V2 a, V2 b) { return (V2){a.x + b.x, a.y + b.y}; }
static V2 v2muls(V2 a, double s) { return (V2){a.x * s, a.y * s}; }
static V2 v2abs(V2 a) { return (V2){fabs(a.x), fabs(a.y)}; }
static V2 v2min(V2 a, V2 b) { return (V2){a.x < b.x ? a.x : b.x, a.y < b.y ? a.y : b.y}; }
static V2 v2max(V2 a, V2 b) { return (V2){a.x > b.x ? a.x : b.x, a.y > b.y ? a.y : b.y}; }
static V2 v2scale(V2 a, V2 b) { return (V2){a.x * b.x, a.y * b.y}; } /* component product */
static V2 v2norm(V2 a) { double l = v2len(a); return l == 0.0 ? a : v2muls(a, 1.0 / l); }
static V2 v2lerp(V2 a, V2 b, double t) { return v2add(a, v2muls(v2sub(b, a), t)); }
static double v2dist(V2 a, V2 b) { return v2len(v2sub(a, b)); }
static double v2angle(V2 a, V2 b) {
  double la = v2len(a), lb = v2len(b);
  if (la == 0.0 || lb == 0.0) return 0.0;
  double c = v2dot(a, b) / (la * lb);
  if (c > 1.0) c = 1.0; if (c < -1.0) c = -1.0;
  return acos(c);
}
static V2 v2clamp_len(V2 a, double maxlen) {
  double l = v2len(a);
  return (l > maxlen && l > 0.0) ? v2muls(a, maxlen / l) : a;
}
static V2 v2project(V2 a, V2 b) {
  double bb = v2dot(b, b);
  return bb == 0.0 ? (V2){0, 0} : v2muls(b, v2dot(a, b) / bb);
}
static V2 v2reject(V2 a, V2 b) { return v2sub(a, v2project(a, b)); }
static V2 v2reflect(V2 a, V2 n) { return v2sub(a, v2muls(n, 2.0 * v2dot(a, n))); }
static V2 v2perp(V2 a) { return (V2){-a.y, a.x}; }
static V2 v2rotate(V2 a, double ang) {
  double c = cos(ang), s = sin(ang);
  return (V2){a.x * c - a.y * s, a.x * s + a.y * c};
}
static V2 v2slerp(V2 a, V2 b, double t) {
  double omega = v2angle(a, b), s = sin(omega);
  if (fabs(s) < 1e-6) return v2lerp(a, b, t); /* v2lerp is unclamped == mfb lerp_unclamped */
  return v2add(v2muls(a, sin((1.0 - t) * omega) / s), v2muls(b, sin(t * omega) / s));
}

/* 3x3 determinant helper for the 3D/4D cross products. */
static double det3(double a, double b, double c,
                   double d, double e, double f,
                   double g, double h, double i) {
  return a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
}

static double cross3_len(double a[3], double b[3]) {
  double cx = a[1] * b[2] - a[2] * b[1];
  double cy = a[2] * b[0] - a[0] * b[2];
  double cz = a[0] * b[1] - a[1] * b[0];
  return sqrt(cx * cx + cy * cy + cz * cz);
}

static double cross4_len(double a[4], double b[4], double c[4]) {
  double d0 = det3(a[1], a[2], a[3], b[1], b[2], b[3], c[1], c[2], c[3]);
  double d1 = -det3(a[0], a[2], a[3], b[0], b[2], b[3], c[0], c[2], c[3]);
  double d2 = det3(a[0], a[1], a[3], b[0], b[1], b[3], c[0], c[1], c[3]);
  double d3 = -det3(a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]);
  return sqrt(d0 * d0 + d1 * d1 + d2 * d2 + d3 * d3);
}

/* ----- coverage: Float family ------------------------------------------ */

static void test_vector_float(void) {
  long long *t = alloc_times();
  double checksum = 0.0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    double acc = 0.0;
    for (int i = 0; i < 20000; i++) {
      double fk = (double)(i - (i / 1000) * 1000);
      V2 a = {fk + 1.0, fk * 0.5 + 2.0};
      V2 b = {fk * 0.25 + 3.0, fk + 1.5};
      V2 nb = v2norm(b);
      acc += v2len(a);
      acc += v2dist(a, b);
      acc += v2dot(a, b);
      acc += v2angle(a, b);
      acc += v2len(v2abs(a));
      acc += v2len(v2min(a, b));
      acc += v2len(v2max(a, b));
      acc += v2len(v2scale(a, b));
      acc += v2len(v2norm(a));
      acc += v2len(v2lerp(a, b, 0.5));
      acc += v2len(v2lerp(a, b, 1.5)); /* lerp_unclamped */
      acc += v2len(v2clamp_len(a, 3.0));
      acc += v2len(v2project(a, b));
      acc += v2len(v2reject(a, b));
      acc += v2len(v2reflect(a, nb));
      acc += v2len(v2slerp(a, b, 0.5));
      acc += v2len(v2perp(a));
      acc += v2len(v2rotate(a, 0.5));
      acc += v2len(v2perp(a)); /* cross(a) 1-ary 2D == perpendicular */
      double a3[3] = {fk + 1.0, fk * 0.5 + 2.0, fk * 0.25 + 3.0};
      double b3[3] = {fk * 0.3 + 1.0, fk + 2.0, fk * 0.7 + 0.5};
      acc += cross3_len(a3, b3);
      double a4[4] = {fk + 1.0, fk * 0.5 + 2.0, fk * 0.25 + 3.0, fk * 0.1 + 1.0};
      double b4[4] = {fk * 0.3 + 1.0, fk + 2.0, fk * 0.7 + 0.5, fk * 0.2 + 2.0};
      double c4[4] = {fk * 0.6 + 1.0, fk * 0.2 + 2.0, fk + 0.5, fk * 0.9 + 1.0};
      acc += cross4_len(a4, b4, c4);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "vector_float = %.3f\n", checksum);
  record("vector", "float", t, RUN);
  free(t);
}

/* ----- coverage: Integer family ---------------------------------------- */

static long rhaz(double x) { return (long)round(x); } /* round half away from zero */

static void test_vector_int(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < 20000; i++) {
      double m = (double)(i - (i / 90) * 90);
      V2 a = {m + 1, m + 2};
      V2 b = {m + 3, m + 5};
      V2 nb = v2norm(b);
      acc += rhaz(v2len(a));
      acc += rhaz(v2dist(a, b));
      acc += rhaz(v2dot(a, b));
      acc += rhaz(v2angle(a, b));
      acc += rhaz(v2len(v2abs(a)));
      acc += rhaz(v2len(v2min(a, b)));
      acc += rhaz(v2len(v2max(a, b)));
      acc += rhaz(v2len(v2scale(a, b)));
      acc += rhaz(v2len(v2norm(a)));
      acc += rhaz(v2len(v2lerp(a, b, 0.5)));
      acc += rhaz(v2len(v2lerp(a, b, 1.5)));
      acc += rhaz(v2len(v2clamp_len(a, 50.0)));
      acc += rhaz(v2len(v2project(a, b)));
      acc += rhaz(v2len(v2reject(a, b)));
      acc += rhaz(v2len(v2reflect(a, nb)));
      acc += rhaz(v2len(v2slerp(a, b, 0.5)));
      acc += rhaz(v2len(v2perp(a)));
      acc += rhaz(v2len(v2rotate(a, 0.5)));
      acc += rhaz(v2len(v2perp(a)));
      double a3[3] = {m + 1, m + 2, m + 3};
      double b3[3] = {m + 4, m + 5, m + 6};
      acc += rhaz(cross3_len(a3, b3));
      double a4[4] = {m + 1, m + 2, m + 3, m + 4};
      double b4[4] = {m + 2, m + 3, m + 4, m + 5};
      double c4[4] = {m + 3, m + 4, m + 5, m + 6};
      acc += rhaz(cross4_len(a4, b4, c4));
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "vector_int = %ld\n", checksum);
  record("vector", "int", t, RUN);
  free(t);
}

void run_vector_group(void) {
  test_vector_math();
  test_vector_float();
  test_vector_int();
}
