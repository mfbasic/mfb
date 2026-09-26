# bug-716: water objects stranded under the toolbar become permanently unselectable after a resize

Last updated: 2026-09-25
Effort: medium (1h-2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: none yet — examples/ has no test harness (see Validation Plan for the proposed gap-filling probe)

`examples/water`'s window can be resized narrower at any time
(`canvas::didResize()`, checked every frame in `examples/water/src/main.mfb:main`
at line 269). Shrinking the window shrinks the play area
(`playWidth(width)`, `examples/water/src/toolbar.mfb:playWidth`), but no code
path ever moves an existing object back inside the new, smaller play area —
only an active drag clamps a position
(`examples/water/src/main.mfb:main`, the `moveShape(...,
math::clamp(...))` call at line 327). If an object was left at an x
coordinate that the toolbar now covers (it was placed there under a wider
window, or a fast double-shrink leaves it there), the object stays visible at
that pixel position but a click there is decided by
`onToolbar(width, at)` before `shapeAt(at.x, at.y)` is ever tried
(`examples/water/src/main.mfb:main`, lines 303-307). Because the click is
routed to the toolbar branch first, `shapeAt` never runs for that pixel and
the object becomes permanently unselectable and unremovable by mouse (it can
still be reached by keyboard if it happens to already be `selected`, but never
re-selected once deselected). **The single correct behavior**: an object's
position should never be left outside the current play area — it must be
clamped back in whenever the play area shrinks (on `didResize`, not only
during a drag), so a click on any object's true screen position is always
decided by `shapeAt` and never swallowed by `onToolbar`.

References:

- `examples/water/src/main.mfb` (the frame loop, resize handling, and mouse
  decision tree)
- `examples/water/src/shapes.mfb` (`shapeAt`, `moveShape`)
- `examples/water/src/toolbar.mfb` (`playWidth`, `onToolbar`, `buttonAt`)
- `examples/wind/src/earth.mfb:clampedView` / `examples/wind/src/main.mfb`
  line 397-404 (`resized(view, size)` calling `clampedView` from the
  `didResize` handler) — a sibling example in the same repo that *does*
  reclamp its view on resize, used below as the working contrast case.

## Failing Reproduction

The real `water` program is `app`-mode (`"mode": "app"` in
`examples/water/project.json`) and requires a live window/canvas, which
cannot be driven headlessly in this environment (confirmed:
`./target/release/mfb build examples/water` builds fine, but running the
`.app` needs an interactive display — not attempted, no claim made about it).

Instead, a standalone console-mode `.mfb` program was built in
`/tmp/bug716-work/repro` that copies the *exact* decision-order logic from
`main.mfb`'s mouse-down branch, using the real `shapeAt` from
`shapes.mfb` (byte-for-byte, only the unused canvas-drawing functions —
`shapeItem`, `shapeItems`, `turnedPoint`, `shapeColor`, and the `IMPORT
canvas`/`IMPORT color` they needed — were removed, since importing `canvas`
at all forces app mode and blocks a headless build) and the real
`playWidth`/`onToolbar`/`buttonAt` from `toolbar.mfb` (same logic; only the
`at AS canvas::Point` parameter was split into `atX, atY AS Float` for the
same reason — the comparison `atX >= playWidth(width)` is unchanged from
`at.x >= playWidth(width)`). Files: `/tmp/bug716-work/repro/src/shapes.mfb`,
`/tmp/bug716-work/repro/src/toolbar.mfb`, `/tmp/bug716-work/repro/src/main.mfb`.

Commands actually run:

```
cd /Users/justinzaun/Development/mfb
./target/release/mfb build /tmp/bug716-work/repro
/tmp/bug716-work/repro/build/bug716repro.out
```

Build output (verbatim):

```
Building bug716repro (executable) for macos-aarch64
Wrote executable to /tmp/bug716-work/repro/build/bug716repro.out
```

Run output (verbatim):

```
old width=300.00 oldPlay(=playWidth)=212.00
placed shape 0 at x=150.00 y=100.00
window shrinks to width=200.00 newPlay(=playWidth)=112.00
shape.x (150.00) >= newPlay (112.00)? TRUE
onToolbar(newWidth, shapeX) = TRUE
REACHED TOOLBAR BRANCH (matches main.mfb line 303-305): buttonAt(newWidth, shapeX, shapeY) = 1 -- shapeAt is NEVER called on this path, so the click on the shape's own pixel is swallowed by the toolbar check.
direct shapeAt(shapeX, shapeY) bypassing onToolbar = 0 (>= 0 means the shape IS visually present and hittable there, but main.mfb's real branch order at line 303 never reaches shapeAt for this click)
```

- Observed: at the shape's own pixel position (150, 100), after the window
  shrinks from 300 to 200 wide, `onToolbar(newWidth, shapeX)` returns `TRUE`
  and the click is decided as a toolbar click (`buttonAt` returns `1`,
  meaning "triangle button", so a real click there would *add a new shape*,
  not select the stranded one). `shapeAt` is never invoked on this path.
- Expected: because the shape is still visually drawn at (150, 100) — nothing
  moved it — a click there should select it. `shapeAt(shapeX, shapeY)` called
  directly (bypassing `onToolbar`, which is not how `main.mfb` actually
  decides) does return `0`, proving the shape is genuinely present and
  hittable; it is only unreachable because of the branch order in
  `main.mfb`'s mouse-down handling.

Contrast case that works correctly today: a click on the same shape *before*
any resize (window still 300 wide, `playWidth(300) = 212 > 150`) goes to the
`ELSE` branch and reaches `shapeAt` normally — `onToolbar` is only wrong once
a shape's `x` and the current `playWidth` have diverged, which only a resize
(or an object placed very close to the edge, `newShapeX`) can cause.

Also a contrast case in a sibling example: `examples/wind` reclamps its
pannable view on every resize (`examples/wind/src/main.mfb:397-404` calls
`resized(view, size)`, which calls `clampedView` per
`examples/wind/src/earth.mfb:205`) — showing the fix pattern (reclamp
position-like state from the resize handler, not only from a drag handler) is
already established practice in this repo's own examples.

| Environment | Details | Result |
| --- | --- | --- |
| macOS aarch64 (headless console repro) | `/tmp/bug716-work/repro` built via `target/release/mfb build`, run directly | fails (matches the documented bug) ✗ |
| Real `examples/water` app, interactive | not run — GUI requires a live window/display | not attempted (no claim made) |

## Root Cause

- `examples/water/src/main.mfb:main`, mouse-down branch (lines 301-316):
  ```
  IF onToolbar(width, at) THEN
    LET button AS Integer = buttonAt(width, at)
    IF button >= 0 THEN selected = addShape(...)
  ELSE
    LET hit AS Integer = shapeAt(at.x, at.y)
    ...
  ```
  `onToolbar` (`examples/water/src/toolbar.mfb:onToolbar`, line 33-35: `RETURN
  at.x >= playWidth(width)`) is checked unconditionally before `shapeAt`
  (`examples/water/src/shapes.mfb:shapeAt`, line 162-167). Any click whose x
  is `>= playWidth(width)` is decided as a toolbar click and `shapeAt` is
  short-circuited — regardless of whether an actual object occupies that
  pixel.
- `examples/water/src/main.mfb:main`, resize handling (lines 268-272):
  ```
  IF canvas::didResize() THEN size = canvas::getSize()
  LET width AS Float = toFloat(size.width)
  LET height AS Float = toFloat(size.height)
  LET play AS Float = playWidth(width)
  pipe = pipeWithin(pipe, play)
  ```
  `play` is recomputed every frame and the pipe is reclamped
  (`pipeWithin`), but no analogous call reclamps any element of `shapes`
  (the `shapes` list itself, mutated only via `shapes.mfb`'s `addShape` /
  `moveShape` / `removeShape` / etc.).
- The only place a shape's position is ever clamped to `play` is the drag
  branch, `examples/water/src/main.mfb:main` line 327:
  ```
  moveShape(selected, math::clamp(at.x + gesture.offsetX, 0.0, play), math::clamp(at.y + gesture.offsetY, 0.0, height))
  ```
  This only runs while `gesture.dragging` is `TRUE` for the currently
  `selected` object — i.e., only as a side effect of the user already having
  successfully grabbed that specific object mid-drag. A shape that has never
  been dragged since the window last shrank, or any shape other than the one
  currently being dragged, is never clamped.
- `examples/water/src/shapes.mfb:moveShape` (line 118-120) itself performs no
  clamping — it is a pure setter; all clamping is the caller's responsibility,
  and `main.mfb`'s resize handler is the only caller that doesn't do it.

Net effect: `playWidth` shrinking without a corresponding reclamp of `shapes`
positions creates x-coordinates that satisfy `onToolbar` while a real object
sits there, and the mouse-down branch order means `shapeAt` — which would
correctly find the object — is never reached for those coordinates.

## Goal

- After any resize (window growing or shrinking), every object's position is
  within the *current* play area (`0 <= x <= playWidth(width)`, `0 <= y <=
  height`), the same bound the drag clamp already enforces
  (`examples/water/src/main.mfb:327`).
- A click on any object's true screen position always reaches `shapeAt` and
  selects it — `onToolbar` never intercepts a click that lands on a real,
  currently-visible object.

### Non-goals (must NOT change)

- The mouse decision order itself (`onToolbar` before `shapeAt`) — this is a
  reasonable priority (toolbar buttons sit on top) as long as objects are
  never actually positioned under the toolbar. The fix should not restructure
  this branch; it should make its precondition (objects never overlap the
  toolbar) hold.
- The drag-time clamp (`main.mfb:327`) and its bound (`0.0` to `play`) —
  unchanged; it is the same target range a resize-time clamp should use.
- Object scale, rotation, spin, or any other per-object state.
- `pipeWithin`'s existing resize-time clamp for the pipe — unaffected, already
  correct, and a model for the fix (`main.mfb:273`).

## Blast Radius

Grep performed: `grep -rn "onToolbar\|shapeAt\|playWidth" examples/` and
`grep -rln "didResize" examples/` and `grep -rn "math::clamp" examples/`
(commands and full output captured above in the investigation; see repo at
HEAD).

- `examples/water/src/main.mfb:main` — the only site with the
  onToolbar-before-shapeAt pattern in the whole `examples/` tree (`onToolbar`,
  `shapeAt` and `playWidth` are all unique to `examples/water`) — fixed by
  this bug.
- `examples/wind/src/main.mfb` (`didResize` handler, line 397) — **unaffected,
  works correctly today**: it calls `resized(view, size)`
  (`examples/wind/src/earth.mfb:205`), which calls `clampedView`
  (`examples/wind/src/earth.mfb:263`) unconditionally on every resize, so the
  view's `cx`/`cy` are always reclamped. This is the pattern water is missing.
- `examples/browser/app/src/main.mfb`, `examples/dungeon/src/main.mfb`,
  `examples/emoji/src/main.mfb`, `examples/ai_chat/src/main.mfb`,
  `examples/bugs/canvas/src/main.mfb` — all call `canvas::didResize()`, but
  none has a toolbar-like region or a list of mouse-selectable objects with
  independent positions (no `math::clamp` of any drag-moved or click-target
  position exists in these files per the grep above except water's own
  `moveShape` clamp) — unaffected, no equivalent hazard.
- `examples/water/src/pipe.mfb:pipeWithin` (defined line 37, called from
  `examples/water/src/main.mfb:main` at line 273) — **already correct**: it is
  invoked unconditionally every frame regardless of resize or drag state,
  which is exactly the missing pattern for `shapes`.

## Fix Design

Add a `SUB clampShapes(play AS Float, height AS Float)` (or equivalent) to
`shapes.mfb` that clamps every shape's `x` to `[0, play]` and `y` to `[0,
height]`, mirroring the bound already used at `main.mfb:327`. Call it from
`main.mfb`'s main loop every frame right after `play`/`height` are computed
(next to the existing `pipe = pipeWithin(pipe, play)` at line 273), not only
inside `IF canvas::didResize() THEN`, so it self-heals even if a shape were
ever left stranded by some other path. This mirrors `pipeWithin`'s existing
unconditional-every-frame pattern rather than gating on `didResize()`, which
keeps the fix simple and avoids a second, resize-only code path to maintain.

Rejected alternative: clamp only inside `IF canvas::didResize() THEN`. Works
for the described case but is more fragile — it silently stops protecting the
invariant if a future change moves a shape by some path other than a drag
(e.g. a programmatic reset), and it duplicates the "clamp every frame
regardless of cause" pattern `pipeWithin` already uses successfully. Rejected
in favor of matching the proven pattern.

Rejected alternative: reorder the mouse-down branch to try `shapeAt` before
`onToolbar`. Rejected — it is a Non-goal above: it would let a real, currently
correctly-positioned toolbar-region hit occasionally be masked if a shape
were dragged all the way to the edge (edge case not currently possible, but
becomes possible without the position invariant), and it does not fix the
underlying problem, which is that a shape's position is allowed to leave the
play area at all.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Land the `/tmp/bug716-work/repro` reproduction (or an equivalent) as a
      committed probe demonstrating the bug (see Validation Plan — examples/
      has no test harness today, so this is new ground).
- [ ] Confirm the blast-radius audit above (done in this document) is
      complete; no further code review needed since `onToolbar`/`shapeAt`/
      `playWidth` are unique to `water`.

Acceptance: the probe fails for the documented reason (shape stranded under
toolbar, unreachable via `onToolbar`-then-`shapeAt`); the audit list above is
complete with a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] Add `clampShapes(play, height)` to `examples/water/src/shapes.mfb`,
      clamping each shape's `x` into `[0, play]` and `y` into `[0, height]`
      via `collections::set`/`WITH`, matching the bound at
      `examples/water/src/main.mfb:327`.
- [ ] Call it unconditionally from `examples/water/src/main.mfb:main`'s main
      loop, next to `pipe = pipeWithin(pipe, play)` (line 273).

Acceptance: re-running the Phase 1 probe (adapted to call the new
`clampShapes`) shows the shape's `x` moved back to `<= play` after the
simulated shrink, and a subsequent `onToolbar`/`shapeAt` check reaches
`shapeAt` and finds it.
Commit: —

### Phase 3 — validation

- [ ] `./scripts/build-examples.sh` (or at minimum `mfb build
      examples/water`) to confirm the change compiles cleanly for every
      target.
- [ ] Re-run the Phase 1/2 probe end-to-end.
- [ ] Manually verify in the real GUI (interactive — not automatable in this
      environment) that resizing the window narrower with an object near the
      right edge leaves it selectable.

Acceptance: `build-examples.sh` succeeds; the probe passes; manual GUI check
confirms (or is explicitly deferred with a note if no interactive display is
available).
Commit: —

## Validation Plan

- Regression test(s): none exist for `examples/` today — this is a gap.
  Proposed: a `tools/water-probe/` standalone `.mfb` program (following the
  `/tmp/bug716-work/repro` structure created for this bug, with a short
  README explaining it exercises `shapes.mfb`/`toolbar.mfb`'s decision logic
  headlessly since the real program needs app mode) that would assert
  `shapeAt` finds every shape after a simulated `clampShapes` call following a
  shrink. Per `AGENTS.md`, this belongs under `tools/<name>/` (a probe, not a
  reusable gate/harness), not `scripts/`, since it targets one bug's logic
  rather than being a general maintenance action; it should be proposed to the
  project rather than added unilaterally within this bug-only task.
- Runtime proof: the `/tmp/bug716-work/repro` reproduction above, re-run after
  the fix with `clampShapes` added, showing `onToolbar` no longer returns
  `TRUE` for the (now-reclamped) shape's position.
- Doc sync: none expected — no man/spec page documents `water`'s internal
  mouse-handling behavior.
- Full suite: `./scripts/build-examples.sh` (examples have no `cargo test`
  coverage; this is the closest thing to a full-suite gate for this file).

## Open Decisions

- Whether `clampShapes` runs every frame (matching `pipeWithin`) or only
  inside `didResize()` — recommended: every frame, per Fix Design above, for
  parity with `pipeWithin` and defense against any future non-drag mutation
  path. No objection raised since this decision was not put to anyone; flagged
  here for the implementer to confirm before Phase 2 lands.
- Whether the proposed `tools/water-probe/` regression probe is worth adding
  given `examples/` has no other precedent for it — recommended: add it, since
  this bug is otherwise unverifiable except by interactive GUI use, but this
  is a call for whoever picks up Phase 1.

## Summary

The bug is a missing invariant, not a broken comparison: `onToolbar` and
`shapeAt` are each correct in isolation, and the drag-time clamp
(`main.mfb:327`) is correct for the one case it covers. The gap is that
nothing clamps a shape's position back into the play area when the play area
itself shrinks outside of an active drag on that exact shape — unlike
`pipeWithin`, which already does this unconditionally every frame, and unlike
`examples/wind`'s `clampedView`, which is called from its `didResize` handler.
The fix is small (one new `SUB` plus one call site) and low-risk; the larger
risk is that `examples/` has no automated regression coverage at all, so
"done" for this bug depends on the interactive GUI check in Phase 3 or on
building the new `tools/` probe.
