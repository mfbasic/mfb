# fractals

A 50,000-dot zoom through five fractals: a Mandelbulb, a Julia set, a Menger
sponge, a quaternion Julia set, and an Apollonian gasket. The dots form a fixed
250 by 200 picture grid. They never move; only their colours change as the
sampled view zooms deeper into each fractal and fades into the next one.

## Build and run

Canvas needs a window, so this is an `--app` build:

```sh
mfb build examples/fractals
```

- macOS — `open examples/fractals/build/fractals.app`
- Linux — `./examples/fractals/build/fractals.AppImage`
- Windows — cross-compile with
  `mfb build --target windows-x86_64 examples/fractals`, then run
  `examples/fractals/build/fractals.exe`

It opens fullscreen. **Esc** quits; there are no other controls.

## How the point clouds are made

- **Mandelbulb** — the `z = 0` plane through the order-eight spherical power
  map.
- **Julia set** — escape-time colours for `z² + (-0.835 - 0.2321i)`.
- **Menger sponge** — the `z = 0` slice, whose base-three removal rule produces
  the familiar carpet.
- **Quaternion Julia** — the `w = 0, z = 0` plane through
  `q² + (-0.2 + 0.7i)`.
- **Apollonian gasket** — the limit bands of a repeated circle-inversion
  system.

The scene list is allocated once. Every frame replaces only the colours of its
50,000 fixed circle slots, presents the picture, and waits for the next drawn
frame—the same high-volume Canvas pattern used by the `sorts` example.
