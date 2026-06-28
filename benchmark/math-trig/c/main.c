/* Forward-trig kernel stress test — C reference (see ../mfb/src/main.mfb). */
#include <stdio.h>
#include <math.h>

int main(void) {
  double acc = 0.0;
  for (int rep = 0; rep < 2000; rep++) {
    double x = 0.001;
    for (int i = 0; i < 1000; i++) {
      acc += sin(x) + cos(x) + tan(x) + atan2(x, 1.0 + x);
      x += 0.0015;
    }
  }
  printf("trig: %.6f\n", acc);
  return 0;
}
