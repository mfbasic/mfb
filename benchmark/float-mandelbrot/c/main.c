/* Counts grid points inside the Mandelbrot set.
 * Grid W x H over real [-2.0, 1.0], imag [-1.5, 1.5]. For each cell center c,
 * iterate z = z*z + c up to MAXITER steps; a cell that never escapes
 * (zr*zr + zi*zi > 4.0) is counted as in-set. */
#include <stdio.h>

int main(void) {
  int w = 600;
  int h = 600;
  int maxiter = 100;
  double wf = (double)w;
  double hf = (double)h;
  long inset = 0;
  for (int y = 0; y < h; y++) {
    double im = -1.5 + 3.0 * ((double)y + 0.5) / hf;
    for (int x = 0; x < w; x++) {
      double re = -2.0 + 3.0 * ((double)x + 0.5) / wf;
      double zr = 0.0;
      double zi = 0.0;
      int escaped = 0;
      int i = 0;
      while (i < maxiter) {
        double nzr = zr * zr - zi * zi + re;
        double nzi = 2.0 * zr * zi + im;
        zr = nzr;
        zi = nzi;
        if (zr * zr + zi * zi > 4.0) {
          escaped = 1;
          i = maxiter;
        } else {
          i = i + 1;
        }
      }
      if (!escaped) {
        inset = inset + 1;
      }
    }
  }
  printf("in-set: %ld\n", inset);
  return 0;
}
