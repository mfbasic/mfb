# fractals

A 25,000-dot flight through five fractals: a Mandelbulb, a Julia set, a
Menger sponge, a quaternion Julia set, and an Apollonian gasket. The camera
flies into each point cloud, eases back while the dots morph into the next
fractal, and repeats forever.

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

- **Mandelbulb** — points that survive ten iterations of the order-eight
  spherical power map.
- **Julia set** — a backward orbit of `z² + (-0.835 - 0.2321i)`, lifted into a
  thin rippled sheet so the camera can pass through it.
- **Menger sponge** — the chaos game over the twenty retained subcubes.
- **Quaternion Julia** — points in the `w = 0` slice that survive twelve
  iterations of `q² + (-0.2 + 0.7i)`.
- **Apollonian gasket** — points on circles obtained by repeatedly reflecting
  one member of a Descartes quadruple; the circles are given a slight depth
  ripple for the flight.

The scene list is allocated once. Every frame replaces its 25,000 circle
slots, sorts them back-to-front, presents the scene, and waits for the next
drawn frame—the same high-volume Canvas pattern used by the `sorts` example.
