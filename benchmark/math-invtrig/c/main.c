/* Inverse-trig kernel stress test — C reference (see ../mfb/src/main.mfb). */
#include <stdio.h>
#include <math.h>

int main(void) {
  double acc = 0.0;
  for (int rep = 0; rep < 2000; rep++) {
    double t = -0.999;
    for (int i = 0; i < 1000; i++) {
      acc += asin(t) + acos(t) + atan(t);
      t += 0.001998;
    }
  }
  printf("invtrig: %.6f\n", acc);
  return 0;
}
