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
  one block per unbroken trail run plus the map and caption items
  (Corrections D2).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-154-C complete | `ls planning/plan-154-C-*` → no match (archived) | MET (2026-09-25: no match; archived to `planning/completed/`) |

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
3. Emit a PolyLine for each run of two or more points, with one colour per
   point: point `k` (newest first) gets the alpha `40 + floor(fade * 195)`
   with `fade = 1 - k / segments`, the ramp the per-segment `Line`s used
   (plan-154-A Corrections A1: the fade is a per-point colour list).

The measured quantity is `blocks=` and the frame time, before and after, on
Metal.

## Phases

> **NOTE — keep the checkboxes current as you go.** **An unticked box means NOT DONE.**

### Phase 1 — adopt and measure

- [x] `flow.mfb`: rewrite `trailItems` as §3 describes, and update its comment.
      The PolyLine is built by `trailPath` and appended INLINE: a first version
      threaded `items` through a helper's parameter and return, which copies the
      whole list per call, and the frame rate fell from 108 to 13 frames in 15 s
      (Corrections D4).
- [x] Take a before/after `--debug` wind run with
      `MFB_CANVAS_STATS=/tmp/wstats.txt` for 15 s on the dev Mac. Record
      `blocks=`, `frames=` and `generations=` in the commit message. Run as
      `bash tools/canvas-bench/bench.sh target/release/mfb wind 15` (headless,
      `MFB_CANVAS_GPU=1`, a `--debug` wind), once `bench.sh`'s wind launch was
      fixed (Corrections D3). Before (`Line` per segment): 108 frames in 15 s,
      last frame `blocks=9618`, `generations=752255` over 116 frames (about 6,485
      builds a frame). After: 99 frames in 15 s, `blocks=981`,
      `generations=65426` over 106 frames (about 617 a frame). Both runs are on
      Metal (`gpuFrames` equals frames). Those two runs were minutes apart
      on a host other sessions were loading, so the frame rates are compared
      again back to back: the old and new `--debug` wind, same compiler,
      launched alternately with a 15 s window after an 8 s warm-up
      (`MFB_CANVAS_STATS` line counts). Before 65 and 64 frames (`blocks=8724`,
      `8420`); after 85 and 85 (`blocks=1159`, `1120`). So the adoption is about
      30% more frames and about 7.5× fewer blocks.
- [x] Launch wind and inspect a screenshot: trails fade, and they don't
      streak across the antimeridian. A GPU frame dumped with
      `MFB_CANVAS_DUMP` from a `--debug` wind after 15 s (`gpuFrames=137
      frames=137`) and converted to PNG: 1,046 tracers, each a trail bright at
      the head fading to faint at the tail, with no seam at a corner and no
      trail across either edge of the map. A 4× crop of the Indian Ocean shows
      the per-point fade along each trail.

Acceptance: `blocks=` is one per unbroken trail run plus the non-trail items,
and the screenshot shows fading trails with no antimeridian streaks.
  Check: the stats run above (est. 2 min) and a screenshot (est. 1 min).
  Result: a probe build (a `/tmp` copy of wind printing the counts per frame)
  measured `particles=1046 map=91 trails=1044..1055` over three frames. So the
  trail items are one per run, within 1% of one per particle: a trail split at
  the antimeridian or the view edge is two runs, and a young particle has no
  run. `blocks=1186` on the same run is those plus the map and the caption
  glyphs. The screenshot check is above.
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

- **D1 (2026-09-24): no `fade` field.** plan-154-A's record carries a per-point
  colour list instead (its Corrections A1), so §3 step 3 was rewritten for it.
- **D2 (2026-09-25): the bound is runs, not particles.** The acceptance said
  `blocks=` is at most the particle count plus the non-trail items. §2 already
  says a skipped segment splits a trail into two PolyLines, so the count can
  exceed the particles. Measured: 1,055 trail items for 1,046 particles. The
  criterion now says what the design produces, one block per unbroken run,
  checked by counting both in a probe.
- **D3 (2026-09-25): `tools/canvas-bench/bench.sh wind` could not run wind.**
  Wind quits at end of input (`readKey` maps EOF to `Quit`), and the bench
  launched it as a background job, whose stdin is `/dev/null`. It drew one frame
  and exited, and the bench reported "no second (first map) frame after 150s"
  twice. Reproduced by hand: `< /dev/null` gives 1 stats line in 12 s, and an
  inherited stdin gives 68. Fixed in both the macOS and Linux paths: stdin is a
  FIFO the launcher holds open for writing, so reads block and never reach EOF.
- **D4 (2026-09-25): a helper that takes and returns the item list is
  quadratic.** The first `trailItems` rewrite appended each run through
  `withRun(items, …) AS List OF DrawItem`. That copies `items` whole on every
  call, the pattern `helper_items.rs`'s module comment measures at 290×. It
  dropped wind to 13 frames in 15 s. The append is now inline in `trailItems`.
- **D5 (2026-09-25): wind's memory growth is bug-690, and this adoption makes
  it faster.** Verifying the Linux bench, wind was SIGKILLed on box 2226 after
  about 250 s, and its RSS climbed about 22 MB/s there. On macOS, release
  builds: the `Line` wind levels off near 4.8 GB at 150 s, and the PolyLine wind
  grows linearly to 11.8 GB. `vmmap` puts the growth in arena chunks
  (`VM_ALLOCATE`), about one 5.3 MB region per frame, a scene copy's size.
  Constant-size PolyLine probes stay flat, and software rendering levels off.
  That is bug-690: a freed block over 2,048 bytes is reused only by a request
  of exactly its size, and a PolyLine scene's size now changes every frame.
  bug-690 is open (HIGH, large) and owns the fix. The evidence was added to
  it. Nothing in the canvas package leaks: every probe's live memory is flat.

## Summary

This is an example change plus the measurement that justifies the feature.
