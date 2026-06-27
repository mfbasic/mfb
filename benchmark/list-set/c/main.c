/* Fixed-width in-place set: build a 200-element int array, then run 10 passes
 * incrementing every element. Prints the checksum (sum of all elements). */
#include <stdio.h>

int main(void) {
  int nums[200];
  for (int i = 0; i < 200; i++) {
    nums[i] = i;
  }

  for (int pass = 0; pass < 10; pass++) {
    for (int j = 0; j < 200; j++) {
      nums[j] = nums[j] + 1;
    }
  }

  long checksum = 0;
  for (int j = 0; j < 200; j++) {
    checksum += nums[j];
  }

  printf("checksum: %ld\n", checksum);
  return 0;
}
