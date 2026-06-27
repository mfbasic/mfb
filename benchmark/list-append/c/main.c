/* Appends 1000 times to a growable int array and a growable string array. */
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
    nums[nums_len] = i;
    nums_len = nums_len + 1;
  }

  char **names = NULL;
  int names_len = 0, names_cap = 0;
  for (int i = 0; i < 1000; i++) {
    char buf[16];
    snprintf(buf, sizeof(buf), "%d", i);
    if (names_len == names_cap) {
      names_cap = names_cap ? names_cap * 2 : 1;
      names = realloc(names, names_cap * sizeof(char *));
    }
    names[names_len] = strdup(buf);
    names_len = names_len + 1;
  }

  printf("ints: %d last=%d\n", nums_len, nums[nums_len - 1]);
  printf("strings: %d last=%s\n", names_len, names[names_len - 1]);
  return 0;
}
