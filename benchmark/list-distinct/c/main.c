/* collections::distinct stress: build a 5000-element array with heavy
 * duplication (i mod 1000), then keep first occurrences in order. The naive
 * O(n^2) "scan the kept list" form is used to mirror mfb's contains()-in-a-loop
 * implementation. Prints the number of distinct values. */
#include <stdio.h>

int main(void) {
  int nums[5000];
  for (int i = 0; i < 5000; i++) {
    nums[i] = i % 1000;
  }

  int unique[5000];
  int unique_len = 0;
  for (int i = 0; i < 5000; i++) {
    int seen = 0;
    for (int k = 0; k < unique_len; k++) {
      if (unique[k] == nums[i]) {
        seen = 1;
        break;
      }
    }
    if (!seen) {
      unique[unique_len++] = nums[i];
    }
  }

  printf("count: %d\n", unique_len);
  return 0;
}
