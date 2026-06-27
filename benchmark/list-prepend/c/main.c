/* Builds a 1000-item list by prepending one item at a time. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(void) {
  int *nums = NULL;
  int nums_len = 0, nums_cap = 0;
  for (int i = 0; i < 1000; i++) {
    if (nums_len == nums_cap) {
      nums_cap = nums_cap ? nums_cap * 2 : 1;
      nums = realloc(nums, nums_cap * sizeof(int));
    }
    memmove(nums + 1, nums, nums_len * sizeof(int));
    nums[0] = i;
    nums_len = nums_len + 1;
  }

  printf("count: %d first=%d\n", nums_len, nums[0]);
  return 0;
}
