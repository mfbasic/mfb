# plan-154-D: `canvas::PolyLine` — wind adopts it, measured

Last updated: 2026-09-24
Effort: small (<1h)
Depends on: plan-154-C

This sub-plan changes `examples/wind/src/flow.mfb:trailItems` to emit one
`canvas::PolyLine` per drawn particle, where it now emits one `canvas::Line`
per segment. It then measures the effect, and runs the whole-feature final
gate.

**Correct behavior.**

- Wind draws the same picture: trails fade head to tail, and a segment is
  skipped where it crosses the antimeridian or lies off screen.
- `MFB_CANVAS_STATS` `blocks=` drops from the 11,291 measured in plan-154-A to
  at most the particle count plus the map and caption items.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-154-C complete | `ls planning/plan-154-C-*` → no match (archived) | NOT MET (2026-09-24) |

If plan-154-C is not complete, this plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**

## 1. Goal

- Wind uses PolyLine.
- The before/after stats are recorded in the commit.
- The full gate passes once for plan-154.

### Non-goals

- **Nothing changes in the swarm model or the tuning constants.**
- **No change to the canvas package** (the earlier letters own it).

## 2. Current State

`trailItems` walks each particle's ring buffer newest-first. It skips a
segment that wraps the antimeridian (`abs(ax - bx) > wrapLimit`) or lies
outside `viewBox`, and emits a `Line` with alpha
`40 + floor(fade * 195)`, where `fade = 1 - k / segments`.

A skipped segment splits a trail. So one particle may need more than one
PolyLine: one per unbroken run.

## 3. Design Overview

For each particle:

1. Walk the ring newest-first, collecting the points of the current run.
2. A wrap or off-screen segment closes the run, and so does the end of the
   ring.
3. Emit a PolyLine for each run of two or more points. Its `fade` reproduces
   the run's share of the head-to-tail ramp, and its paint alpha is the
   run's first-point alpha.

The measured quantity is `blocks=` and the frame time, before and after, on
Metal.

## Phases

> **NOTE — keep the checkboxes current as you go.** **An unticked box means NOT DONE.**

### Phase 1 — adopt and measure

- [ ] `flow.mfb`: rewrite `trailItems` as §3 describes, and update its comment.
- [ ] Take a before/after `--debug` wind run with
      `MFB_CANVAS_STATS=/tmp/wstats.txt` for 15 s on the dev Mac. Record
      `blocks=`, `frames=` and `generations=` in the commit message.
- [ ] Launch wind and inspect a screenshot: trails fade, and they don't
      streak across the antimeridian.

Acceptance: `blocks=` is at most the particle count plus the non-trail items,
and the screenshot shows fading trails with no antimeridian streaks.
  Check: the stats run above (est. 2 min) and a screenshot (est. 1 min).
Commit: —

## Validation Plan

- Runtime proof: the stats lines and the screenshot.
- Final gate (once, for all of plan-154):
  `scripts/test-accept.sh target/debug/mfb target/accept-actual`,
  `cargo test`, and `scripts/test-canvas-vulkan.sh target/release/mfb --box 2228`
  (est. 50 min; the full suite is the only run that sees every golden a canvas
  runtime change can move — `tests/syntax/app/app-mouse-surface` carries
  canvas `.ncodesum` goldens, per `.ai/testing-gates.md`).

## Open Decisions

- None.

## Corrections

(none yet)

## Summary

This is an example change plus the measurement that justifies the feature.
