# water-collision-bench

Regression and benchmark tool for bug-717: `examples/water/src/shapes.mfb`'s
`contactWith` used to pay two `math::cos`/`math::sin` calls before any cheap
reject, on every one of up to `POOL_SIZE * shapeCount * SUBSTEPS` calls a
frame, and a star shape paid 40 more inside `starContact`, recomputing its ten
corners from scratch on every call.

The fix adds two things to `examples/water/src/shapes.mfb`:

- an exact per-kind bounding-radius reject in `contactWith`, before the
  rotation into the shape's own frame (which is what needs the trig) —
  a drop nowhere near a shape now costs no trig call;
- a per-shape-per-frame cache of `cos(angle)`/`sin(angle)` and, for a star,
  its ten corner coordinates (`rebuildShapeCache`, called once at the top of
  `stepWater`, after `spinShapes` has settled every shape's angle for the
  frame), read back by `contactWithCached` and `starContactCached` instead of
  recomputed on every one of the hot loop's calls.

`contactWith`'s own signature, and its return value for every input, are
unchanged — `shapeAt` (one call per mouse click) still calls it exactly as
before. `contactWithCached`/`rebuildShapeCache`/`starContactCached` are new,
additional functions `stepWater`'s hot path uses instead.

## What this tool checks

`src/shapes_under_test.mfb` is `examples/water/src/shapes.mfb` with its
canvas-drawing section removed (collision testing needs no display, so this
stays a console-mode program). `src/main.mfb` re-implements the *pre-fix*
`contactWith` body verbatim as `oracleContactWith` — unconditional trig, no
bounding-radius reject — reusing the kind-tests (`rectangleContact`,
`triangleContact`, `circleContact`, `ellipseContact`, `starContact`) that the
fix did not touch.

It builds every shape kind at three scales and four angles, and for each one
tests an 9x9 grid of points spanning well outside the shape's reach to well
inside it, comparing `oracleContactWith`'s result against both the fixed
`contactWith` and the new `contactWithCached` — bit-for-bit (`hit`, `x`, `y`,
`nx`, `ny`) — and reports any mismatch. It also times the oracle path against
the fixed cached path over the same grid, repeated, and prints both —
informational only, not asserted (wall-clock timing on a shared machine is
noisy; the bug doc measured a 1.8x spread within one variant of its own
benchmark).

## Running it

```
$ target/release/mfb build tools/water-collision-bench
$ ./tools/water-collision-bench/build/water-collision-bench.out
shapes: 60
points tested per shape: 81
total comparisons: 4860
contacts found (oracle): 1535
mismatches: 0
oracle path elapsed ms: 64.05
fixed cached path elapsed ms: 32.37
PASS: water-collision-bench — contactWith and contactWithCached agree with the pre-fix oracle on every shape and point tested
```

Against `shapes_under_test.mfb` at the pre-fix HEAD (no `contactWithCached`/
`rebuildShapeCache`), this tool fails to build at all — `main.mfb` calls
functions that do not exist yet, which is the RED this tool proves against
before the fix landed.
