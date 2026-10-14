/* GROUP: convert — the C oracle for convert.mfb (plan-87 Theme 3, in-memory
 * number-conversion throughput). A pure parse+render loop, no file IO.
 *
 * Expected checksums: int=5000438890, float=624993751111120. The float row uses
 * v = i/8 (an exact dyadic rational), so %.6f rendering is exact and unambiguous
 * across the three languages. The value fold rounds to nearest (+0.5 before the
 * cast) to match mfb, whose naive digit-accumulation toFloat is not correctly
 * rounded (e.g. "2.375000" -> one ULP low); the parse error is far under 0.5
 * micro-units, so rounding recovers the exact i*125000 in every language. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "convertbench.h"

#define CONV_N 100000

/* ----- convert int: render "%d" + reparse strtoll ------------------------- */

static void test_convert_int(void) {
  long long *t = alloc_times();
  long long checksum = 0;
  char buf[32];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long long acc = 0;
    for (int i = 0; i < CONV_N; i++) {
      int n = snprintf(buf, sizeof buf, "%d", i);  /* toString(i) */
      long long back = strtoll(buf, NULL, 10);     /* toInt(s) */
      acc += back + n;                             /* n == strlen(buf) == len(s) */
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "convert_int = %lld\n", checksum);
  record("convert", "int", t, RUN);
  free(t);
}

/* ----- convert float: render "%.6f" + reparse strtod ---------------------- */

static void test_convert_float(void) {
  long long *t = alloc_times();
  long long checksum = 0;
  char buf[64];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long long acc = 0;
    for (int i = 0; i < CONV_N; i++) {
      double v = (double)i / 8.0;                    /* exact dyadic rational */
      int n = snprintf(buf, sizeof buf, "%.6f", v);  /* toString(v, 6) */
      double b = strtod(buf, NULL);                  /* toFloat(s) */
      acc += (long long)(b * 1000000.0 + 0.5) + n;   /* round -> i*125000 + len */
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "convert_float = %lld\n", checksum);
  record("convert", "float", t, RUN);
  free(t);
}

void run_convert_group(void) {
  test_convert_int();
  test_convert_float();
}
