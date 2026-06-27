/* Copies a 1000-item string list 1000 times, then a 1000-item record list. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define N 1000

typedef struct {
  int n;
  char *s;
} Rec;

int main(void) {
  char buf[16];

  /* Build the string list once. */
  char **strs = malloc(N * sizeof(char *));
  for (int i = 0; i < N; i++) {
    snprintf(buf, sizeof buf, "%d", i);
    strs[i] = strdup(buf);
  }
  long acc = 0;
  for (int i = 0; i < N; i++) {
    char **c = malloc(N * sizeof(char *));
    memcpy(c, strs, N * sizeof(char *)); /* flat copy, like the collection block */
    acc = acc + N;
    free(c);
  }

  /* Build the record list once. */
  Rec *recs = malloc(N * sizeof(Rec));
  for (int i = 0; i < N; i++) {
    snprintf(buf, sizeof buf, "%d", i);
    recs[i].n = i;
    recs[i].s = strdup(buf);
  }
  long acc_recs = 0;
  for (int i = 0; i < N; i++) {
    Rec *c = malloc(N * sizeof(Rec));
    memcpy(c, recs, N * sizeof(Rec));
    acc_recs = acc_recs + N;
    free(c);
  }

  printf("strings: %ld recs: %ld\n", acc, acc_recs);
  return 0;
}
