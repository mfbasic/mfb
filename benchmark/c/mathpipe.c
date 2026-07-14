/* GROUP: float (matmul) + mathpipe (dft, stats) — the C oracle for
 * mathpipe.mfb's float/transcendental pipelines.
 *
 *   matmul  — dense 64x64 Float matmul in the exact mfb i,j,k loop order and
 *             element formulas, so the %.6f checksum is bit-identical (64054.589366).
 *   dft     — naive O(N^2) two-tone DFT; checksum is the two dominant bin
 *             indices (best1*1000+best2 = 5020), robust to low-bit sin/cos gaps.
 *   stats   — mean/variance over x[i]=i%1000, both exact IEEE (499.5, 83333.25).
 *
 * finance (Money) is mfb-only and has no row here. */
#include <math.h>
#include <stdio.h>
#include <stdlib.h>

#include "bench.h"
#include "mathpipe.h"

void test_matmul(void) {
  const int n = 64;
  double *a = malloc((size_t)n * n * sizeof(double));
  double *b = malloc((size_t)n * n * sizeof(double));
  for (int i = 0; i < n; i++)
    for (int j = 0; j < n; j++) {
      a[i * n + j] = (double)((i * 7 + j * 3) % 97) / 97.0;
      b[i * n + j] = (double)((i * 5 + j * 13) % 89) / 89.0;
    }
  long long *t = alloc_times();
  double checksum = 0.0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    double sum = 0.0;
    for (int i = 0; i < n; i++)
      for (int j = 0; j < n; j++) {
        double acc = 0.0;
        for (int k = 0; k < n; k++) acc += a[i * n + k] * b[k * n + j];
        sum += acc;
      }
    checksum = sum;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "matmul = %.6f\n", checksum);
  record("float", "matmul", t, RUN);
  free(t);
  free(a);
  free(b);
}

static void test_dft(void) {
  const int n = 256;
  const double PI = 3.141592653589793;
  const double two_pi = 2.0 * PI;
  double *sig = malloc((size_t)n * sizeof(double));
  for (int tt = 0; tt < n; tt++) {
    double tf = (double)tt;
    sig[tt] = cos(two_pi * 5.0 * tf / (double)n) + 0.5 * cos(two_pi * 20.0 * tf / (double)n);
  }
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    int best1 = 0, best2 = 0;
    double mag1 = -1.0, mag2 = -1.0;
    for (int k = 0; k < n / 2; k++) {
      double re = 0.0, im = 0.0, kf = (double)k;
      for (int tt = 0; tt < n; tt++) {
        double ang = two_pi * kf * (double)tt / (double)n;
        double s = sig[tt];
        re += s * cos(ang);
        im -= s * sin(ang);
      }
      double mag = re * re + im * im;
      if (mag > mag1) {
        mag2 = mag1;
        best2 = best1;
        mag1 = mag;
        best1 = k;
      } else if (mag > mag2) {
        mag2 = mag;
        best2 = k;
      }
    }
    checksum = (long)best1 * 1000 + best2;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "dft = %ld\n", checksum);
  record("mathpipe", "dft", t, RUN);
  free(t);
  free(sig);
}

static void test_stats(void) {
  const int n = 200000;
  double *xs = malloc((size_t)n * sizeof(double));
  for (int i = 0; i < n; i++) xs[i] = (double)(i % 1000);
  long long *t = alloc_times();
  double mean = 0.0, variance = 0.0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    double sum = 0.0;
    for (int i = 0; i < n; i++) sum += xs[i];
    mean = sum / (double)n;
    double sq = 0.0;
    for (int i = 0; i < n; i++) {
      double d = xs[i] - mean;
      sq += d * d;
    }
    variance = sq / (double)n;
    (void)sqrt(variance); /* stddev — exercised but not in the checksum */
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "stats = %.6f,%.6f\n", mean, variance);
  record("mathpipe", "stats", t, RUN);
  free(t);
  free(xs);
}

void run_mathpipe_group(void) {
  test_dft();
  test_stats();
}
