/* Builds a list of 50 random integers, then copies and sorts it SORTS times. */
#include <stdio.h>
#include <stdlib.h>

#define N 50

static int cmp(const void *a, const void *b) {
  int x = *(const int *)a, y = *(const int *)b;
  return (x > y) - (x < y);
}

int main(void) {
  int sorts = 1;
  int base[N];
  for (int i = 0; i < N; i++) {
    base[i] = rand() % 1000001;
  }

  long checksum = 0;
  int tmp[N];
  for (int k = 0; k < sorts; k++) {
    for (int i = 0; i < N; i++) {
      tmp[i] = base[i];
    }
    qsort(tmp, N, sizeof(int), cmp);
    checksum = checksum + tmp[0];
  }
  (void)checksum;

  int ok = 1;
  for (int i = 1; i < N; i++) {
    if (tmp[i] < tmp[i - 1]) {
      ok = 0;
    }
  }

  printf("count: %d sorted: %d\n", N, ok);
  return 0;
}
