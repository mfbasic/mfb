/* GROUP: math (math:: package coverage)
 *
 * Moved out of main.c into its own translation unit; shared timing/recording
 * infra comes from bench.h. See main.c for the suite overview.
 *
 * Two kinds of coverage live here, mirroring math.mfb:
 *   1. The individual libm kernels (sin, cos, ... sqrt), each run 2000 x 1000
 *      times — the historical benchmark surface.
 *   2. Consolidated "coverage" rows (float, int, simd) that exercise the
 *      remaining math:: members across the Float, Integer and array surfaces.
 *      (The Fixed row is mfb-only and has no C equivalent.) */
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "bench.h"
#include "mathbench.h"

/* ----- individual libm kernels (each run 2000 x 1000 times) ------------- */

#define MATH_KERNEL(fnname, label, expr, init, step)         \
  static void fnname(void) {                                 \
    long long *t = alloc_times();                            \
    double checksum = 0.0;                                   \
    for (int r = 0; r < RUN; r++) {                          \
      long long t0 = now_ns();                               \
      double acc = 0.0;                                      \
      for (int rep = 0; rep < 2000; rep++) {                 \
        double v = (init);                                   \
        for (int i = 0; i < 1000; i++) {                     \
          acc += (expr);                                     \
          v += (step);                                       \
        }                                                    \
      }                                                      \
      checksum = acc;                                        \
      t[r] = now_ns() - t0;                                  \
    }                                                        \
    fprintf(stderr, "%s = %.6f\n", label, checksum);         \
    record("math", label, t, RUN);                           \
    free(t);                                                 \
  }

MATH_KERNEL(test_sin, "sin", sin(v), 0.001, 0.0015)
MATH_KERNEL(test_cos, "cos", cos(v), 0.001, 0.0015)
MATH_KERNEL(test_tan, "tan", tan(v), 0.001, 0.0015)
MATH_KERNEL(test_atan2, "atan2", atan2(v, 1.0 + v), 0.001, 0.0015)
MATH_KERNEL(test_asin, "asin", asin(v), -0.999, 0.001998)
MATH_KERNEL(test_acos, "acos", acos(v), -0.999, 0.001998)
MATH_KERNEL(test_atan, "atan", atan(v), -0.999, 0.001998)
MATH_KERNEL(test_exp, "exp", exp(v * 0.1), 0.001, 0.005)
MATH_KERNEL(test_log, "log", log(v), 0.001, 0.005)
MATH_KERNEL(test_log10, "log10", log10(v), 0.001, 0.005)
MATH_KERNEL(test_pow, "pow", pow(v, 1.5), 0.001, 0.005)
MATH_KERNEL(test_sqrt, "sqrt", sqrt(v), 0.001, 0.005)

/* ----- coverage: Float (abs/floor/ceil/round/min/max/clamp) ------------- */

static double dclamp(double v, double lo, double hi) {
  return v < lo ? lo : (v > hi ? hi : v);
}

static void test_math_float(void) {
  long long *t = alloc_times();
  double checksum = 0.0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    double acc = 0.0;
    for (int i = 0; i < 200000; i++) {
      double v = (double)i * 0.5 - 50000.0;
      acc += fabs(v);
      acc += floor(v * 0.001);
      acc += ceil(v * 0.001);
      acc += round(v * 0.001); /* round-half-away, matches math::round */
      acc += (v < 0.0 ? v : 0.0);
      acc += (v > 0.0 ? v : 0.0);
      acc += dclamp(v, -10.0, 10.0);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "math_float = %.3f\n", checksum);
  record("math", "float", t, RUN);
  free(t);
}

/* ----- coverage: Integer (abs/min/max/clamp + a seeded PRNG loop) -------
 *
 * mfb seeds its PCG64 generator; C rand() and PCG diverge, so we use our own
 * deterministic LCG seeded to the same constant. Cross-language checksum
 * parity for this row is therefore not expected — only reproducibility. */
static uint64_t lcg_state;
static void lcg_seed(uint64_t s) { lcg_state = s; }
static long lcg_rand(long lo, long hi) {
  lcg_state = lcg_state * 6364136223846793005ULL + 1442695040888963407ULL;
  uint64_t x = lcg_state >> 33;
  long span = hi - lo + 1;
  return lo + (long)(x % (uint64_t)span);
}

static long lclamp(long v, long lo, long hi) {
  return v < lo ? lo : (v > hi ? hi : v);
}

static void test_math_int(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    lcg_seed(123456789ULL);
    for (int i = 0; i < 200000; i++) {
      long v = (long)i - 100000;
      acc += (v < 0 ? -v : v);
      acc += (v < 0 ? v : 0);
      acc += (v > 0 ? v : 0);
      acc += lclamp(v, -10, 10);
      acc += lcg_rand(0, 100);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "math_int = %ld\n", checksum);
  record("math", "int", t, RUN);
  free(t);
}

/* ----- coverage: SIMD (array/element-wise math over freshly built lists) - */

#define SIMD_N 1024

static double *math_range(int n, double lo, double span) {
  double *xs = malloc((size_t)n * sizeof(double));
  for (int i = 0; i < n; i++) xs[i] = lo + (double)i / (double)n * span;
  return xs;
}

static void test_math_simd(void) {
  double *unit = math_range(SIMD_N, -0.9, 1.8);   /* [-0.9, 0.9] */
  double *pos = math_range(SIMD_N, 0.01, 4.0);    /* (0, ~4]     */
  double *big = math_range(SIMD_N, -1000.0, 2000.0);
  double *expo = math_range(SIMD_N, -2.0, 4.0);
  double *lo = math_range(SIMD_N, -5.0, 10.0);
  long long *t = alloc_times();
  double checksum = 0.0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    double acc = 0.0;
    for (int rep = 0; rep < 200; rep++) {
      double sAbs = 0, sFloor = 0, sCeil = 0, sRound = 0, sMin = 0, sMax = 0;
      double sClamp = 0, sSqrt = 0, sLog = 0, sLog10 = 0, sExp = 0;
      double sSin = 0, sCos = 0, sTan = 0, sAtan = 0, sAsin = 0, sAcos = 0;
      double sPow = 0, sAtan2 = 0;
      for (int i = 0; i < SIMD_N; i++) {
        sAbs += fabs(big[i]);
        sFloor += floor(pos[i]);
        sCeil += ceil(pos[i]);
        sRound += round(pos[i]);
        sMin += (big[i] < lo[i] ? big[i] : lo[i]);
        sMax += (big[i] > lo[i] ? big[i] : lo[i]);
        sClamp += dclamp(big[i], -1.0, 1.0);
        sSqrt += sqrt(pos[i]);
        sLog += log(pos[i]);
        sLog10 += log10(pos[i]);
        sExp += exp(expo[i]);
        sSin += sin(unit[i]);
        sCos += cos(unit[i]);
        sTan += tan(unit[i]);
        sAtan += atan(unit[i]);
        sAsin += asin(unit[i]);
        sAcos += acos(unit[i]);
        sPow += pow(pos[i], expo[i]);
        sAtan2 += atan2(unit[i], pos[i]);
      }
      acc += sAbs + sFloor + sCeil + sRound + sMin + sMax + sClamp + sSqrt +
             sLog + sLog10 + sExp + sSin + sCos + sTan + sAtan + sAsin +
             sAcos + sPow + sAtan2;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "math_simd = %.3f\n", checksum);
  record("math", "simd", t, RUN);
  free(unit); free(pos); free(big); free(expo); free(lo); free(t);
}

void run_math_group(void) {
  test_sin(); test_cos(); test_tan(); test_atan2();
  test_asin(); test_acos(); test_atan();
  test_exp(); test_log(); test_log10(); test_pow(); test_sqrt();
  test_math_float();
  test_math_int();
  test_math_simd();
}
