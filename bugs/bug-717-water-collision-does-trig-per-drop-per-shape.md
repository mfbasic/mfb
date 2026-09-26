# bug-717: water's collision test spends unconditional trig on every drop x shape x substep

Last updated: 2026-09-25
Effort: medium (1h-2h)
Severity: LOW
Class: Other

Status: Open
Regression Test: none yet — no perf-benchmark harness exists for `examples/`; see Validation Plan

`examples/water/src/shapes.mfb:contactWith` is the collision test the water
example's physics loop calls for every drop, every substep, every shape, every
frame (`examples/water/src/water.mfb:stepWater`, the `FOR k = 0 TO count - 1`
loop at line 181 nested inside the drop loop and the `SUBSTEPS` loop). With the
example's own tuning (`POOL_SIZE = 2000`, `SUBSTEPS = 3`), a frame with 20
shapes and every drop out of the pool calls `contactWith` up to 120,000 times.
`contactWith` computes `math::cos(s.angle)` and `math::sin(s.angle)`
unconditionally, before any cheap distance check that could skip the rest of
the work for a drop nowhere near the shape, and for a star-shaped object the
per-kind test (`starContact`) recomputes all ten of the star's corner
coordinates — 40 trig calls — on every single invocation, even though the
corners are the same for that shape until its angle or scale next changes.
None of this is wrong: shapes tested this way, `shapeAt`
(`examples/water/src/shapes.mfb:162-167`), returns the correct hit. It is only
slower than it needs to be — the water example still runs at its stated frame
rate on the hardware it targets (unmeasured whether it ever drops frames), so
this is a performance bug, not a correctness one.

**Correct behavior.** A drop far outside a shape's reach costs O(1) cheap
arithmetic, no trig; `math::cos`/`math::sin` of a shape's own angle are paid at
most once per shape per frame (the angle does not change within a frame — see
Root Cause), not once per drop that reaches that shape; and a star's corner
coordinates are computed once per shape per frame (or cached until the angle
or scale actually changes) rather than fully recomputed inside every
`starContact` call.

References:

- `examples/water/src/water.mfb:stepWater` (lines 158-210) — the hot loop.
- `examples/water/src/shapes.mfb:contactWith` (lines 186-205), `:starContact`
  (lines 329-380), `:starCornerX`/`:starCornerY`/`:starAngle`/`:starRadius`
  (lines 302-317) — the code under test.
- Found while reviewing `examples/water` for an unrelated task (no prior bug
  or plan doc covers this path).

## Failing Reproduction

This is a perf bug: "failing" means the measured trig-call volume and wall
time below, not a crash or wrong output. Built and run today
(`target/release/mfb` from this checkout, no rebuild):

```
target/release/mfb build /tmp/bug717-work/bench_before
/tmp/bug717-work/bench_before/build/bench_before.out
```

`/tmp/bug717-work/bench_before/src/main.mfb` is `contactWith` and every
function it calls (`rectangleContact`, `circleContact`, `ellipseContact`,
`triangleContact`/`outside`, `starContact`/`starCornerX`/`starCornerY`/
`starAngle`/`starRadius`), copied verbatim from
`examples/water/src/shapes.mfb`, plus a console driver with no `canvas`/`app`
dependency. The driver mirrors `stepWater`'s loop nesting exactly: `N_DROPS =
2000`, `M_SHAPES = 20` (4 of each of the 5 kinds, so 4 stars), `FRAMES = 30`,
`SUBSTEPS = 3` — `2000 * 20 * 30 * 3 = 3,600,000` total `contactWith` calls,
timed with `datetime::monotonicNanos()`.

Observed (six runs, `time <binary>`, this machine):

```
contactWith calls: 3600000
hits: 25650
elapsed ms: 3497.77   (wall 3.693s, `time`: 0.67s user 0.01s sys 18% cpu)
elapsed ms: 5576.87
elapsed ms: 6250.88
elapsed ms: 6110.78
elapsed ms: 3652.48
elapsed ms: 3457.55
```

- `hits` (contacts found) is identical across every run: 25650. The workload
  is deterministic; only wall time varies.
- The six-run spread is wide (3457.77-6250.88ms, min-to-max ratio ~1.8x) —
  this machine's timing is noisy at this granularity, most likely background
  load rather than anything in the benchmark (the low `cpu%` in the `time`
  output, 11-18%, backs that: the process spends most of its wall time not
  running). Treat the absolute numbers as noisy; the before/after comparison
  below is the load-bearing measurement, and it does not overlap.
- Star-corner count, verified by reading the code rather than assumed: the
  claim that a `starContact` call does 40 trig evaluations is **confirmed
  correct**, not just repeated. `starContact` loops `k = 0 TO corners - 1`
  where `corners = STAR_POINTS * 2 = 10` (line 333), and each iteration calls
  `starCornerX(k)`, `starCornerY(k)`, `starCornerX((k+1) MOD corners)`,
  `starCornerY((k+1) MOD corners)` (lines 341-344) — 4 corner-coordinate calls
  per iteration, each one trig call (`starCornerX` is `starRadius(k) *
  math::cos(starAngle(k))`, one `cos`; `starCornerY` is one `sin`; line
  302-308). 10 iterations x 4 calls = 40 trig calls per `starContact`
  invocation. Each of the star's 10 corners is visited twice (once as the
  edge's start, once as the next edge's end), so the 40 is 10 corners x 2
  visits x {cos, sin} = 40, not an approximation.
- Including the 2 trig calls `contactWith` itself makes for every shape
  (`math::cos`/`math::sin` of `s.angle`, lines 189-190), one `contactWith`
  call against a star costs 42 trig evaluations end to end.

Second build, the fix candidate (not the proposed production fix — see Fix
Design — built only to measure the achievable gain):

```
target/release/mfb build /tmp/bug717-work/bench_after
/tmp/bug717-work/bench_after/build/bench_after.out
```

Same file, with one addition to `contactWith`: before computing `c`/`n`
(`math::cos`/`math::sin`), compute a per-kind circumscribing radius
(`maxRadiusOf`, exact for every kind — rectangle's diagonal, triangle's base
corner, the circle/ellipse/star radius itself) and reject the drop with a
squared-distance comparison against `(radius * scale + reach)^2` using the
already-computed `dx`/`dy` (distance from centre is rotation-invariant, so no
trig is needed for this check). No other line changed.

Observed (six runs):

```
contactWith calls: 3600000
hits: 25650
elapsed ms: 2129.59
elapsed ms: 1489.27
elapsed ms: 1496.17
elapsed ms: 1563.94
elapsed ms: 1399.23
elapsed ms: 1310.06
```

- `hits: 25650` on every run — identical to the before variant. The
  bounding-radius reject changed no observable outcome in this benchmark.
- Every "after" run (max 2129.59ms) is faster than every "before" run (min
  3457.55ms) — the two distributions do not overlap, despite the noise inside
  each. Median before ≈4614.7ms, median after ≈1492.7ms: **roughly a 3x
  speedup** on this workload (measured range across the two medians and the
  min/min comparison: ~2.6x-3.9x). This quantifies the ceiling on the
  bounding-radius part of the fix alone; it does not include the separate
  gain available from caching `cos`/`sin` per shape per frame or from caching
  star corners (neither is in this probe).
- `math::cos`/`math::sin` are not independently profilable with the tools
  used here (no `perf`/sampling profiler run as part of this bug) — the
  before/after wall-time delta is the only measurement taken, not a
  trig-call-count-attributed cost. Treat "how much of the 3x is the trig
  itself vs. the rest of `starContact`'s work" as a **guess**: most of it,
  because for the 16 of 20 shapes (non-star kinds) the entire remaining
  per-kind test is 2-6 comparisons and at most one `math::sqrt`, so skipping 2
  trig calls plus that test dominates; for star shapes the reject also skips
  40 trig calls plus the corner/edge geometry, which should dominate further,
  but this was not measured separately per kind.

## Root Cause

Two independent mechanisms, both in `examples/water/src/shapes.mfb`:

1. **Trig before any reject, every call.**
   `contactWith` (lines 186-205) computes `c := math::cos(s.angle)` and `n :=
   math::sin(s.angle)` at lines 189-190 to rotate the point into the shape's
   frame, unconditionally, before dispatching to the per-kind test (lines
   195-201). Every per-kind test *does* reject cheaply once inside the
   shape's frame (`rectangleContact` line 212 `IF ax >= hx OR ay >= hy`,
   `circleContact` line 226 `IF d2 >= r * r`, `ellipseContact` line 242 `IF f
   >= 1.0`, `starContact` line 331 `IF lx*lx+ly*ly >= far*far`) — but reaching
   any of those rejects already cost the two trig calls, because the
   rotation the reject is computed in *requires* them. There is no reject
   before line 189 using only `dx`/`dy` (lines 187-188), even though the
   distance from the shape's centre — which is what every one of those
   per-kind bounds tests, once rotated — does not depend on the rotation at
   all.

2. **Star corners recomputed from scratch every call, twice each.**
   `starContact` (lines 329-380) loops over all `corners = STAR_POINTS * 2 =
   10` corner indices (line 333/340) and for each iteration calls
   `starCornerX`/`starCornerY` (lines 302-308, each one `math::cos`/
   `math::sin` of `starAngle(k)`, lines 310-311) for both the current corner
   `k` and the next corner `(k+1) MOD corners` (lines 341-344) — so corner `k`
   is recomputed once as a "current" corner and again, on the previous
   iteration, as a "next" corner. Nothing about a star's corners in its own
   frame depends on the point being tested; they depend only on the shape's
   `scale` (`STAR_OUTER`/`STAR_INNER` are constants, `starAngle` doesn't take
   `scale` either — the `* scale` multiply happens at the call site, lines
   341-344).

**Is the angle recomputation cacheable per shape per frame?** Yes, checked by
reading the caller chain: `examples/water/src/main.mfb` calls `spinShapes(elapsed)`
(line 337) once, then `stepWater(elapsed, ...)` (line 339) once, per frame.
`spinShapes` (`examples/water/src/shapes.mfb:140-145`) is the only place that
mutates `s.angle`, and it runs to completion — updating every shape once —
*before* `stepWater` starts its drop/substep/shape loop. Within one
`stepWater` call, `shapes` is read-only (confirmed: `stepWater` and
`spreadWater` never call `collections::set` on `shapes`), so `s.angle` (and
`s.scale`) are constant for every one of the up-to-120,000 `contactWith` calls
made against that shape in that frame. A per-shape, per-frame cache of
`cos(angle)`/`sin(angle)` (and, for stars, of the 10 corner coordinates) would
be exact, not an approximation — nothing changes between the calls it would
be reused across.

## Goal

- A drop whose distance from a shape's centre exceeds that shape's
  circumscribing radius plus `reach` costs no `math::cos`/`math::sin` call.
- `math::cos(s.angle)`/`math::sin(s.angle)` are computed at most once per
  shape per frame that shape is tested against any drop, not once per
  `contactWith` call.
- A star shape's 10 corner coordinates are computed at most once per shape per
  frame (or cached until the shape's `angle`/`scale` next changes), not fully
  recomputed inside every `starContact` call.
- `contactWith`'s return value (`hit`, `x`, `y`, `nx`, `ny`) is bit-for-bit
  unchanged for every input it is called with today.

### Non-goals (must NOT change)

- Collision correctness or the shape of any contact (position, normal, which
  edge/corner it resolves to). The bounding-radius reject must be an exact
  upper bound for every kind (verified for the benchmark's `maxRadiusOf`:
  rectangle's diagonal `sqrt(hw^2+hh^2)`, triangle's base-corner distance
  `sqrt(half^2+below^2)` which is >= the apex distance `above`, ellipse's
  `max(a,b)`, circle's and star's own radius) — a bound that ever
  underestimates would silently drop real contacts, which is the tempting
  wrong fix to avoid (an approximate/cheaper-but-unsound bound).
  `TRIANGLE_ABOVE = 60.0 < sqrt(70^2+30^2) = 76.16`, so the triangle's true
  farthest vertex is the base corner, not the apex — using the smaller `above`
  distance alone would be exactly that unsound shortcut.
- `Shape`/`Contact` record layouts, the `contactWith` signature, or any other
  public surface of `shapes.mfb`/`water.mfb`.
- The example's visual behavior, frame pacing, or tuning constants
  (`POOL_SIZE`, `SUBSTEPS`, etc.).

## Blast Radius

Actual search, not from memory:

```
grep -rn "contactWith\|starContact\|starCornerX\|starCornerY" examples/water/src/*.mfb examples/**/*.mfb
```

- `examples/water/src/shapes.mfb:shapeAt` (line 164) — calls `contactWith`
  once per shape per mouse click, not per frame; negligible cost regardless
  of this bug. Unaffected in practice, but shares the same `contactWith` code
  path so the fix benefits it for free.
- `examples/water/src/water.mfb:stepWater` (line 182) — the hot path this bug
  targets; fixed by this bug.
- No other `.mfb` file anywhere under `examples/` references `contactWith`,
  `starContact`, `starCornerX`, or `starCornerY` (grep above returns exactly
  these two files).
- Other unconditional per-frame trig in `examples/`, checked for the same
  hazard (trig inside an O(N x M) nested loop with a cheap reject available
  but not taken): `examples/wind/src/flow.mfb:stepSwarm` (line 353,
  `math::cos(radians(p.lat))`) is O(particles) per frame, one trig call per
  particle, with no inner shape/object loop and no bounding check to hoist it
  past — different shape of problem, out of scope. `examples/water/src/toolbar.mfb`
  (line 71), `examples/bugs/canvas/src/main.mfb`, `examples/emoji/src/main.mfb`,
  `examples/wind/src/earth.mfb` use `math::cos`/`math::sin` for decorative
  per-frame drawing or map-projection math, none of them inside a nested
  drops-x-shapes (or equivalent N x M) collision loop — unaffected.

## Fix Design

Two changes to `examples/water/src/shapes.mfb`, kept independent so either can
land alone:

1. **Bounding-radius early-out in `contactWith`**, before the `math::cos`/
   `math::sin` calls: add a `maxRadiusOf(kind)`-style per-kind circumscribing
   radius (exact upper bound, per the Non-goals constraint above) and reject
   using `dx`/`dy` (already computed) before rotating into the shape's frame.
   This is the change measured in the benchmark above (~3x on that workload,
   noisy). Rejected alternative: an approximate/average radius per kind —
   rejected because it is not provably an upper bound and risks dropping real
   contacts near a rotated rectangle's corner or a star's point.
2. **Per-shape-per-frame cache for `cos(angle)`/`sin(angle)` and, for stars,
   the 10 corner coordinates.** Where the cache lives is an implementation
   choice for whoever picks this up — e.g. a parallel `List OF Float` per
   cached quantity, indexed like `shapes`, recomputed once at the top of
   `stepWater` (after `spinShapes` has run and before the drop loop) rather
   than inside `contactWith`. Rejected alternative: caching inside `Shape`
   itself (adding `cosAngle`/`sinAngle`/corner fields to the `TYPE Shape`
   record) — rejected because it would need every mutator
   (`rotateShape`, `scaleShape`, `spinShapes`) to keep the cache in sync,
   spreading the invalidation logic across more call sites than a
   frame-scoped side table recomputed once per `stepWater` call.

Both changes are pure performance work: `contactWith`'s return value must be
identical before and after, which is what the benchmark's matching `hits:
25650` across every run (both variants) demonstrates for its fixed input, and
what the real fix's validation must demonstrate for the shipped code path.

## Phases

### Phase 1 — failing benchmark + audit (no behavior change)

- [x] Build the /tmp benchmark (`bench_before`) reproducing `contactWith`'s
      current trig-call volume and timing under the water example's own
      loop shape. Confirmed the 40-trig-call `starContact` claim by reading
      the code, not by assumption.
- [x] Complete the blast-radius audit above; every site has a verdict.

Acceptance: benchmark runs today and shows the trig-before-reject cost;
audit lists every `contactWith`/`starContact` caller and the sibling
per-frame-trig sites, with a verdict for each.
Commit: — (benchmark lives in `/tmp/bug717-work`, not committed; see
Validation Plan for where a landed version would go).

### Phase 2 — the fix

- [ ] Add the bounding-radius early-out to `contactWith`
      (`examples/water/src/shapes.mfb`), using an exact per-kind
      circumscribing radius.
- [ ] Add the per-shape-per-frame cache for `cos(angle)`/`sin(angle)` and the
      star's 10 corner coordinates, recomputed once per `stepWater` call
      after `spinShapes` runs.
- [ ] Confirm `contactWith`'s return value is unchanged for the shapes the
      example already exercises (manual play-test: pour water on a rotated,
      scaled star and every other kind; compare against the pre-fix build).

Acceptance: the example still looks and behaves identically; the Phase 1
benchmark, re-pointed at the fixed `shapes.mfb`, shows the same class of
speedup measured in Phase 1's `/tmp` probe.
Commit: —

### Phase 3 — validation

- [ ] `scripts/build-examples.sh` — the water example still builds for every
      target.
- [ ] Re-run the Phase 1 benchmark against the fixed `contactWith` (copied
      into the same `/tmp` harness, or wherever Phase 2 lands a permanent
      regression benchmark — see Validation Plan) and record the before/after
      delta in this file.
- [ ] Manual play-test of `examples/water` on at least one platform: pour
      water on every shape kind, rotated and scaled, spinning and still;
      confirm no visual change from the pre-fix build.

Acceptance: `scripts/build-examples.sh` succeeds; the benchmark shows the
predicted speedup against the real fix; the manual play-test shows no
behavior change.
Commit: —

## Validation Plan

- Regression test(s): none exist yet. `examples/` has no test harness today
  (confirmed: no `tests/` directory under `examples/water`, and
  `scripts/build-examples.sh` only builds, it does not run or benchmark). A
  regression benchmark for this fix would need a new home — most naturally a
  console-mode probe under `tools/water-collision-bench/` (per the
  `tools/<name>/` convention for benchmarks/oracles in `AGENTS.md`), built
  from a copy of `shapes.mfb`'s collision functions the way this bug's `/tmp`
  probe is, with a README explaining what it measures and why. That is a
  design decision for whoever picks up Phase 2/3, not made here.
- Runtime proof: the `/tmp/bug717-work/bench_before` vs. `bench_after`
  comparison in this bug, re-run against the real fixed
  `examples/water/src/shapes.mfb` in Phase 3.
- Doc sync: none expected — `shapes.mfb`'s own header comment already
  describes the contact-finding contract at the level this fix operates
  under (it changes performance, not the contract).
- Full suite: `scripts/build-examples.sh` (this is an `examples/`-only
  change; the compiler's own `cargo test` suite does not exercise example
  source).

## Summary

Confirmed by reading `examples/water/src/shapes.mfb:contactWith` (lines
186-205) and `:starContact` (lines 329-380): `contactWith` pays two trig
calls before any reject on every call, and a star shape pays 40 more inside
`starContact` (verified by counting the loop structure, not assumed from the
prompt — the claim was correct). A `/tmp` benchmark built and run today
against `target/release/mfb` shows a bounding-radius early-out alone (no
cos/sin caching, no star-corner caching) gives roughly a 3x wall-time
reduction on a 2000-drop x 20-shape x 3-substep x 30-frame workload
(3,600,000 `contactWith` calls), though the underlying timing on this machine
was noisy enough (~1.8x spread within each variant) that only the gap between
the two distributions, not either distribution's absolute numbers, should be
trusted. The remaining, unmeasured part of the fix — caching `cos`/`sin` and
the star's corners per shape per frame — is confirmed valid by tracing
`spinShapes`/`stepWater`'s call order in `examples/water/src/main.mfb` (angle
and scale are frame-invariant inside one `stepWater` call) but its own gain
was not benchmarked separately. The real engineering risk in landing this is
keeping the bounding radius an exact (never-underestimating) upper bound per
kind, not the caching itself, which the call-order trace shows is safe.
