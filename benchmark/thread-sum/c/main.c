/* Sums 0..39,999,999 by splitting into four 10,000,000 chunks, each summed on
 * its own pthread (real parallelism). */
#include <stdio.h>
#include <pthread.h>

#define CHUNK 10000000L

typedef struct {
  long start;
  long result;
} Arg;

static void *sum_chunk(void *p) {
  Arg *a = p;
  long total = 0;
  for (long i = a->start; i < a->start + CHUNK; i++) {
    total += i;
  }
  a->result = total;
  return NULL;
}

int main(void) {
  pthread_t th[4];
  Arg args[4];
  for (int k = 0; k < 4; k++) {
    args[k].start = (long)k * CHUNK;
    pthread_create(&th[k], NULL, sum_chunk, &args[k]);
  }
  long total = 0;
  for (int k = 0; k < 4; k++) {
    pthread_join(th[k], NULL);
    total += args[k].result;
  }
  printf("total: %ld\n", total);
  return 0;
}
