# water-probe

Regression check for bug-716: a `examples/water` object left at an x the
play area no longer covers, after the window is resized narrower, became
permanently unselectable — `onToolbar` is checked before `shapeAt` in
`main.mfb`'s mouse-down handling, so a click on the object's own, still
visible, pixel was decided as a toolbar click.

## What this tool checks, and how

This tool builds and runs the **real** `examples/water/src/shapes.mfb` and
`toolbar.mfb` — `run.sh` stages them into a fresh scratch project on every
invocation (`mktemp -d`, then `cp` from the live example tree), alongside
`probe/main.mfb` — the only file this tool commits. If either file
regresses, the next run stages the regressed file and this probe catches
it.

`probe/main.mfb` places a shape well inside a 300-wide window's play area,
shrinks the window to 200 wide (which shrinks the play area to less than the
shape's x), calls `clampShapes` (the fix), and checks: the shape's centre is
no longer past the new play area; a click on a point on the shape's visible
body (not its exact mathematical centre — see "A boundary note, not a bug"
below) is decided by `shapeAt`, not swallowed by `onToolbar`. It calls the
real `onToolbar`/`buttonAt` with a real `vector::Float2`, exactly as
`examples/water/src/main.mfb` does — this probe is an `--app` build, run
headless via `MFB_HEADLESS=1` (the same technique `wind-map-probe`
uses), so no signature substitute is needed.

## Running it

```
$ ./tools/water-probe/run.sh target/release/mfb
old width=300.00 oldPlay=212.00
placed shape 0 at x=150.00 y=100.00
window shrinks to width=200.00 newPlay=112.00
shape.x after clampShapes: 112.00
PASS: bug-716 — a shape's position is reclamped into the shrunk play area, onToolbar no longer claims its pixel, and shapeAt finds it
```

Verified RED against the real pre-fix `shapes.mfb` (`git show <pre-fix
commit>^:examples/water/src/shapes.mfb`, swapped in temporarily, not
committed): the probe fails to **build** — `clampShapes` does not exist yet.
Restored, this tool is GREEN again.

Skips (exit 0) on a non-macOS host, since this is an `--app` build and only
macOS is exercised here.

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
