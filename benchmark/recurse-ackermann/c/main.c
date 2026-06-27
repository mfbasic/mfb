/* Ackermann function — deeply nested recursive call/return overhead. */
#include <stdio.h>

static int ack(int m, int n) {
  if (m == 0) {
    return n + 1;
  }
  if (n == 0) {
    return ack(m - 1, 1);
  }
  return ack(m - 1, ack(m, n - 1));
}

int main(void) {
  int result = ack(3, 7);
  printf("ack(3,7): %d\n", result);
  return 0;
}
