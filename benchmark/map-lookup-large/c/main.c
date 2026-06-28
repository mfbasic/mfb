/* 20000-item integer hash map build + lookup, mirroring the mfb benchmark. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define N 20000
#define CAP 32768 /* power of two >= 2*N */

static long keys[CAP];
static long vals[CAP];
static char used[CAP];

static size_t probe(long k) {
  size_t h = (size_t)(k * 1099511628211UL) & (CAP - 1);
  while (used[h] && keys[h] != k) h = (h + 1) & (CAP - 1);
  return h;
}

int main(void) {
  for (long i = 0; i < N; i++) {
    size_t h = probe(i);
    used[h] = 1; keys[h] = i; vals[h] = i;
  }
  long sum = 0;
  for (long i = 0; i < N; i++) {
    size_t h = probe(i);
    sum += vals[h];
  }
  printf("count: %d sum: %ld\n", N, sum);
  return 0;
}
