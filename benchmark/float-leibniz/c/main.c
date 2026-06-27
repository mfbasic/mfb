/* Approximates pi via the Leibniz series:
 *   pi = 4 * sum_{k=0..N-1} (-1)^k / (2k+1)
 * accumulated as a double with an alternating sign. */
#include <stdio.h>

int main(void) {
  double total = 0.0;
  double sign = 1.0;
  for (int k = 0; k < 1000000; k++) {
    double denom = (double)(2 * k + 1);
    total = total + sign / denom;
    sign = sign * -1.0;
  }
  double pi = 4.0 * total;
  printf("pi: %.5f\n", pi);
  return 0;
}
