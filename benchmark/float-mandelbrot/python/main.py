"""Counts grid points inside the Mandelbrot set.

Grid W x H over real [-2.0, 1.0], imag [-1.5, 1.5]. For each cell center c,
iterate z = z*z + c up to MAXITER steps; a cell that never escapes
(zr*zr + zi*zi > 4.0) is counted as in-set.
"""


def main():
    w = 600
    h = 600
    maxiter = 100
    wf = float(w)
    hf = float(h)
    inset = 0
    for y in range(h):
        im = -1.5 + 3.0 * (float(y) + 0.5) / hf
        for x in range(w):
            re = -2.0 + 3.0 * (float(x) + 0.5) / wf
            zr = 0.0
            zi = 0.0
            escaped = False
            i = 0
            while i < maxiter:
                nzr = zr * zr - zi * zi + re
                nzi = 2.0 * zr * zi + im
                zr = nzr
                zi = nzi
                if zr * zr + zi * zi > 4.0:
                    escaped = True
                    i = maxiter
                else:
                    i = i + 1
            if not escaped:
                inset = inset + 1
    print("in-set: " + str(inset))
    return 0


if __name__ == "__main__":
    main()
