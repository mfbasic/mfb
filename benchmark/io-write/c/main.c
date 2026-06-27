/* Writes 100000 lines to stdout: the integers 0..99999, each on its own line. */
#include <stdio.h>

int main(void) {
  for (int i = 0; i < 100000; i++) {
    printf("%d\n", i);
  }
  return 0;
}
