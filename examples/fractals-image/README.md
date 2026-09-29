# fractals-image

An image-buffer version of [`fractals`](../fractals/README.md). It keeps the same
fractal sampling, colours, zoom, and crossfades, but writes the picture into a
250 × 200 RGBA byte buffer instead of constructing separate circle items.

```sh
mfb build examples/fractals-image
open examples/fractals-image/build/fractals-image.app
```

On Linux, run `examples/fractals-image/build/fractals-image.AppImage`.
For Windows, build with `mfb build --target windows-x86_64 examples/fractals-image`
and run `examples/fractals-image/build/fractals-image.exe`.

It opens fullscreen; **Esc** quits. The image stretches with the window. Unlike
the original's separated dots, the pixels fill the picture continuously.

## Image updates

`main` creates a reusable `List OF Byte` and a `canvas::Image`. Each frame writes
RGB channels into the buffer; alpha stays opaque. `canvas::setBytes` updates the
image, and a single `canvas::Picture` displays it alongside the title and status
text. Fractal calculations are the same as the original, so this change targets
scene construction and rendering overhead.

## Comparing performance

From the repository root on macOS:

```sh
python3 tools/fractals-benchmark/compare.py
```

The [benchmark README](../../tools/fractals-benchmark/README.md) describes the
fixed workloads, colour checks, timing boundaries, and raw results.
