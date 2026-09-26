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

## What this tool checks, and how

This tool builds and runs the **real** `examples/water/src/shapes.mfb` —
`run.sh` stages it into a fresh scratch project on every invocation
(`mktemp -d`, then `cp` from the live example tree), alongside
`probe/main.mfb` — the only file this tool commits. If `shapes.mfb`
regresses, the very next run stages the regressed file and this probe
catches it; a committed fork could not (and previously didn't — see below).

### The frozen oracle

`probe/main.mfb` re-implements the *pre-fix* `contactWith` body verbatim as
`oracleContactWith` (unconditional trig, no bounding-radius reject), reusing
the kind-tests (`rectangleContact`, `triangleContact`, `circleContact`,
`ellipseContact`, `starContact`) the fix did not touch — from the real,
staged `shapes.mfb`, not a copy. `oracleContactWith` itself is deliberately
**frozen**: it exists to answer one fixed question — does the real, current
`contactWith`/`contactWithCached` still agree with the pre-fix behavior — not
to track `shapes.mfb`, so it is correct for it to never change, unlike a
probe that forks the file under test wholesale (which would silently stop
tracking a regression).

It builds every shape kind at three scales and four angles, and for each one
tests an 9x9 grid of points spanning well outside the shape's reach to well
inside it, comparing `oracleContactWith`'s result against both the real
`contactWith` and the real `contactWithCached` — bit-for-bit (`hit`, `x`, `y`,
`nx`, `ny`) — and reports any mismatch. It also times the oracle path against
the fixed cached path over the same grid, repeated, and prints both —
informational only, not asserted (wall-clock timing on a shared machine is
noisy; the bug doc measured a 1.8x spread within one variant of its own
benchmark).

## Running it

```
$ ./tools/water-collision-bench/run.sh target/release/mfb
shapes: 60
points tested per shape: 81
total comparisons: 4860
contacts found (oracle): 1535
mismatches: 0
oracle path elapsed ms: 63.53
fixed cached path elapsed ms: 28.10
PASS: water-collision-bench — contactWith and contactWithCached agree with the pre-fix oracle on every shape and point tested
```

Verified RED against the real pre-fix `shapes.mfb` (`git show <pre-fix
commit>^:examples/water/src/shapes.mfb`, swapped in temporarily, not
committed): the probe fails to **build** — `contactWithCached` and
`rebuildShapeCache` do not exist yet. Restored, this tool is GREEN again.

Skips (exit 0) on a non-macOS host: the staged project is `"mode": "app"`
(required to `IMPORT canvas`/`color`, which `shapes.mfb`'s drawing section
needs even though this probe never calls a mode-gated canvas function), and
only macOS is exercised here.
