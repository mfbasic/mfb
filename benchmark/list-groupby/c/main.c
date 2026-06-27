/* collections::groupBy stress: group a 2000-element array into 100 buckets
 * (i mod 100), appending each item to its bucket's list. Prints the number of
 * groups. */
#include <stdio.h>
#include <stdlib.h>

#define KEYS 100

int main(void) {
  int *bucket[KEYS];
  int len[KEYS] = {0};
  int cap[KEYS] = {0};

  for (int i = 0; i < 2000; i++) {
    int k = i % KEYS;
    if (len[k] == cap[k]) {
      cap[k] = cap[k] ? cap[k] * 2 : 1;
      bucket[k] = realloc(len[k] ? bucket[k] : NULL, cap[k] * sizeof(int));
    }
    bucket[k][len[k]++] = i;
  }

  int groups = 0;
  for (int k = 0; k < KEYS; k++) {
    if (len[k] > 0) {
      groups++;
    }
  }

  printf("groups: %d\n", groups);
  return 0;
}
