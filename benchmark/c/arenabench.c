/* GROUP: arena — the C oracle for arena.mfb. C's allocator has no arena free-
 * list degradation, so these just reproduce the mfb loop arithmetic exactly
 * (with real malloc'd temporaries for comparable work). Checksums must match:
 *   transient=18597, mixed=30000, growshrink=4000 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "arenabench.h"

static void test_arena_transient(void) {
  const char *base = "abcdefghij"; /* len 10 */
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < 400; i++) {
      int size = (i % 16) + 1;
      long *tmp = malloc((size_t)size * sizeof(long));
      for (int k = 0; k < size; k++) tmp[k] = k;
      long s = 0;
      for (int k = 0; k < size; k++) s += tmp[k];
      acc += s; /* collections::sum(tmp) */
      free(tmp);
      int slicelen = (i % 7) + 1;
      char *sl = malloc((size_t)slicelen + 1);
      memcpy(sl, base, (size_t)slicelen);
      sl[slicelen] = '\0';
      acc += (long)strlen(sl); /* len(strings::left(base, slicelen)) */
      free(sl);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "arena_transient = %ld\n", checksum);
  record("arena", "transient", t, RUN);
  free(t);
}

static void test_arena_mixed(void) {
  long *longLived = malloc(1000 * sizeof(long));
  for (int i = 0; i < 1000; i++) longLived[i] = i;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < 400; i++) {
      int size = (i % 20) + 1;
      long *tmp = malloc((size_t)size * sizeof(long));
      for (int k = 0; k < size; k++) tmp[k] = k;
      long s = 0;
      for (int k = 0; k < size; k++) s += tmp[k];
      acc += s;
      free(tmp);
      int reps = (i % 5) + 1; /* strings::repeat("ab", reps) */
      char *str = malloc((size_t)reps * 2 + 1);
      for (int k = 0; k < reps; k++) memcpy(str + (size_t)k * 2, "ab", 2);
      str[reps * 2] = '\0';
      acc += (long)strlen(str);
      free(str);
    }
    acc += 1000; /* len(longLived) */
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "arena_mixed = %ld\n", checksum);
  record("arena", "mixed", t, RUN);
  free(t);
  free(longLived);
}

static void test_arena_growshrink(void) {
  const int grow = 100, cycles = 200;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int cycle = 0; cycle < cycles; cycle++) {
      long *xs = malloc((size_t)grow * sizeof(long));
      for (int k = 0; k < grow; k++) xs[k] = k;
      int headn = 10 < grow ? 10 : grow;               /* take(xs, 10) */
      int tailn = grow - (grow - 10);                  /* drop(xs, grow-10) */
      acc += headn + tailn;
      free(xs);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "arena_growshrink = %ld\n", checksum);
  record("arena", "growshrink", t, RUN);
  free(t);
}

void run_arena_group(void) {
  test_arena_transient();
  test_arena_mixed();
  test_arena_growshrink();
}
