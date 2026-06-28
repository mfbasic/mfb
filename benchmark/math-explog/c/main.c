/* Exp/log/power kernel stress test — C reference (see ../mfb/src/main.mfb). */
#include <stdio.h>
#include <math.h>

int main(void) {
  double acc = 0.0;
  for (int rep = 0; rep < 2000; rep++) {
    double v = 0.001;
    for (int i = 0; i < 1000; i++) {
      acc += exp(v * 0.1) + log(v) + log10(v) + pow(v, 1.5);
      v += 0.005;
    }
  }
  printf("explog: %.6f\n", acc);
  return 0;
}
