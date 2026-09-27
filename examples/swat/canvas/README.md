# swat-canvas

Swat — click the bugs, spare the plants. The canvas edition of
[`examples/swat/tui`](../tui): the same game, drawn with shapes (circles,
ellipses, lines, polygons, rounded rectangles) instead of text, on a windowed
`--app` build.

## Build and run

Canvas needs a window, so this is a `--app` build:

```sh
mfb build examples/swat/canvas
```

- macOS — `open examples/swat/canvas/build/swat-canvas.app`
- Linux — `./examples/swat/canvas/build/swat-canvas.AppImage`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/swat/canvas`,
  then run `examples/swat/canvas/build/swat-canvas.exe`

## Rules

Plants sprout at random (at most `level * 10` at once; the oldest wilts when
a new one would go over that). A bug appears somewhere on the field and stays
for `10 / level + 1` seconds before scurrying off, returning somewhere new
5–10 seconds later. Clicking the bug scores +1 and splats it; clicking a
plant scores −2. A score below zero ends the game; every 15 points is
another level, each with its own bug and plants on its own ground. After the
tenth level the cycle starts again.

## Controls

- **Mouse click** — swat whatever is under the cursor
- **Esc** — pause (press again to resume)
- **R** — restart
- **Q** — quit

No dependencies beyond `app`, `canvas`, `color`, `datetime`, `math`,
`strings`, `collections`.
