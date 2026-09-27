# water

A pipe pours water down the window; it falls, splashes off whatever is in its
way, and drains and recirculates. A toolbar on the right adds shapes
(rectangle, triangle, circle, ellipse, star) that can be moved, turned, spun,
resized and removed. See the header comment in
[`src/main.mfb`](src/main.mfb:1-29) for the full picture.

## Build and run

Canvas needs a window, so this is a `--app` build:

```sh
mfb build examples/water
```

- macOS — `open examples/water/build/water.app`
- Linux — `./examples/water/build/water.AppImage`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/water`,
  then run `examples/water/build/water.exe`

## Controls

- **Drag the pipe** — slide it along the top of the window
- **Click the pipe** — turn the water on or off
- **Click a toolbar button** — add that shape
- **Click / drag a shape** — select it / move it
- **A / D** — turn the selected shape anticlockwise / clockwise, or stop it
  spinning if it already is
- **Q / E** — set the selected shape spinning anticlockwise / clockwise (the
  same key again stops it)
- **W / S** — grow / shrink the selected shape
- **Delete / Backspace** — remove the selected shape
- **Esc** — quit

Arrow keys are documented in the source as equivalent to W A S D, but the
current windowed build does not deliver them to the program (bug-699, the
keyboard-input rework, is in progress); use the letter keys above.

No dependencies beyond `app`, `canvas`, `color`, `math`, and the other
standard packages the program imports.
