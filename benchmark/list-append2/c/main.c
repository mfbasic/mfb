/* Builds a 1000-item list by appending a 10-item list 100 times. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(void) {
  int ten[10] = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9};

  int *nums = NULL;
  int nums_len = 0, nums_cap = 0;
  for (int i = 0; i < 100; i++) {
    if (nums_len + 10 > nums_cap) {
      nums_cap = nums_cap ? nums_cap * 2 : 10;
      while (nums_cap < nums_len + 10) {
        nums_cap = nums_cap * 2;
      }
      nums = realloc(nums, nums_cap * sizeof(int));
    }
    memcpy(nums + nums_len, ten, 10 * sizeof(int));
    nums_len = nums_len + 10;
  }

  printf("count: %d last=%d\n", nums_len, nums[nums_len - 1]);
  return 0;
}
