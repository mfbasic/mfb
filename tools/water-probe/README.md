# water-probe

Regression check for bug-716: a `examples/water` object left at an x the
play area no longer covers, after the window is resized narrower, became
permanently unselectable — `onToolbar` is checked before `shapeAt` in
`main.mfb`'s mouse-down handling, so a click on the object's own, still
visible, pixel was decided as a toolbar click.

The real `water` program is `--app`-mode and needs a live window, which
cannot be driven headlessly in every environment. This tool is a console-mode
program built from verbatim copies of the relevant logic:

- `src/shapes_under_test.mfb` — `examples/water/src/shapes.mfb` with its
  canvas-drawing section removed (collision/position logic needs no
  display), including the bug-716 fix (`clampShapes`) and the bug-717 fix
  (the bounding-radius reject and per-shape cache) it was copied alongside.
- `src/toolbar_under_test.mfb` — `examples/water/src/toolbar.mfb`'s decision
  logic (`playWidth`/`onToolbar`/`buttonAt`/`toolKind`), with `at AS
  canvas::Point` split into `atX, atY AS Float` parameters (importing
  `canvas` at all forces `--app` mode) — the same change this bug's own
  manual reproduction made.
- `src/main.mfb` — places a shape well inside a 300-wide window's play area,
  shrinks the window to 200 wide (which shrinks the play area to less than
  the shape's x), calls `clampShapes` (the fix), and checks: the shape's
  centre is no longer past the new play area; a click on a point on the
  shape's visible body (not its exact mathematical centre — see the file's
  own comment for why that distinction matters) is decided by `shapeAt`, not
  swallowed by `onToolbar`.

## Running it

```
$ target/release/mfb build tools/water-probe   # once, or whenever mfb itself changes
$ ./tools/water-probe/run.sh target/release/mfb
old width=300.00 oldPlay=212.00
placed shape 0 at x=150.00 y=100.00
window shrinks to width=200.00 newPlay=112.00
shape.x after clampShapes: 112.00
PASS: bug-716 — a shape's position is reclamped into the shrunk play area, onToolbar no longer claims its pixel, and shapeAt finds it
```

Without the fix (`clampShapes` not called — checked by hand while landing this
tool, not committed as a separate mode): `shape.x after clampShapes: 150.00`,
then two failures — the shape is still past `newPlay`, and a click on its
body is still swallowed by the toolbar.

## A boundary note, not a bug

Clamping a *stranded* object's centre to `newPlay` always lands the centre
coordinate exactly on `onToolbar`'s own boundary (`atX >= playWidth(width)`)
— that is inherent to clamping *to* an upper bound whenever the pre-clamp
value exceeded it, not a defect in the fix. It is the same single-pixel
ambiguity the pre-existing, unchanged drag clamp
(`main.mfb`'s mouse-drag handler, also clamped to `[0, play]`) shares at that
exact coordinate, and it is not what a real click hits: a click lands
somewhere on the object's visible body, most of which is now inside the play
area. That is why this tool tests a point on the body, not the centroid.
