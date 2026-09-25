# plan-155-B: `canvas::getFrameStats()`

Last updated: 2026-09-24
Effort: large (3h–1d)
Depends on: plan-155-A

Add `canvas::getFrameStats() AS canvas::FrameStats`. It returns one
consistent snapshot of how often the program presents, how often the backend
actually draws, how long the last drawn frame took to render, which renderer
drew it, and the display's refresh rate.

It lets a program measure its own step (`dt`) and see the gap between the
frames it offers and the frames that get drawn (`pps` versus `fps`), without
reading the clock itself.

**Correct behavior.** For a program that does
`present; getFrameStats()` in a loop:

- `presents` counts every `present` and `presentLayers` call, including
  identical re-presents.
- `frames` counts frames the backend drew. It does not count wake-ups where
  nothing had changed.
- `dt` is the seconds between the last two presents, and 0 before the second.
- `pps` and `fps` are rates over the trailing one second. Both fall to 0 within
  one second of the activity stopping.
- `displayHz` is plan-155-A's value, or 0 if it is unknown.

References:

- plan-155-A (the display rate, and its §3 storage).
- `.ai/canvas-threading.md` — §2 (arena state is per-thread, which is why
  every counter here lives in process-global graphics state), §3 (`present`
  ordering), §4 (a static scene costs zero frames), §10 (renderer choice).
- `.ai/man-content.md` — the man text for the record, the enum and the function.
- `mfb spec app canvas` — "Rendering conventions" (the renderer is chosen per
  frame, with fallback).

## Prerequisites

See plan-155-A for the feature-wide prerequisites.

| Must be true | Command | Status |
|---|---|---|
| plan-155-A complete | `ls planning/plan-155-A-*` → no match (archived) | NOT MET |
| `displayMilliHz` exists | `grep -n '"displayMilliHz"' src/codegen/builtins/canvas/func_graphics.rs` → 1 match | NOT MET |

If plan-155-A is not complete, this plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue, and again before you decide to stop.

## 1. Goal

- `canvas::FrameStats`, `canvas::Renderer` and `canvas::getFrameStats()` are
  public, documented, and their man examples run.
- Each field obeys the "Correct behavior" list above, pinned by runtime tests.

### Non-goals (explicit constraints)

- **`present` stays non-blocking.** Race-matrix rows R5 and R6 hold as
  written. The only work added to `present` is one mutex-guarded counter and
  timestamp store.
- **No redraw trigger is added.** Reading stats never causes a frame.
- **The frame skip is unchanged.** An identical present still publishes
  nothing, and it is still counted in `presents`.
- **No new stats-line field**, except as needed to test (see Phase 2).

## 2. Current State

**Worker globals are invisible to the graphics thread.** `__CANVAS_FRAMES`,
`__CANVAS_SKIPPED` and `__CANVAS_GPU_FRAMES` are MFBASIC globals. Under §2 they
are per-thread, so the worker cannot read the graphics thread's copy.

Only `GRAPHICS_OFFSET_FRAMES` (200) is process-global. It is incremented by
`runtime/canvas/mod.rs:emit_frame_done` under `GRAPHICS_OFFSET_MUTEX`, and it
counts **every** loop iteration, skipped ones included:
`helper_render.rs:__canvas_renderFrame` returns early when
`len(damage) = 0`, and `__canvas_renderLoop` still calls
`canvas::frameDone()`.

**The mutex exists only once the graphics thread has started.**
`emit_start_graphics` runs `pthread_mutex_init` and `pthread_cond_init`, and
`__canvas_ensureGraphics` calls it only from `present`'s
`IF installed OR moved` branch (`func_present.rs:BODY`). So any new reader or
writer must check `GRAPHICS_OFFSET_STARTED` before it locks.

**The renderer is chosen per frame.** `__canvas_renderFrame` tries Metal
(`useGpu() AND metalReady()`), then Vulkan, then software. A declined frame
falls back. `useGpu()` is the program's request, not what drew the frame.

**`present`'s wrapper** (`func_present.rs:BODY`, `__canvas_present`), in
order:

1. reclaim loop;
2. `carriedHashes`;
3. `publishScene`;
4. group signature;
5. `IF installed OR moved`: `publishHashes`, `ensureGraphics`,
   `signalRedraw`, `syncFrame`.

`presentLayers` has its own wrapper (`func_present_layers.rs`).

**Records.** A record is returned natively the way
`func_get_size.rs:lower_get_size` does it: arena-alloc, store the fields,
pointer in `RESULT_VALUE_REGISTER`. Or it is built in MFBASIC from internal
builtins, the way `helper_surface.rs:__canvas_surfaceSize` does. Enums are
registered like `CapStyle` in `canvas/mod.rs:register`, and variant order
fixes the ordinal.

**The timer.** `canvas::frameNanos` (`func_frame_nanos.rs`) is an internal
monotonic clock in nanoseconds.

### Measured populations

| What | Count | Command |
|---|---|---|
| present wrappers that must record a present | 2 | `grep -ln "canvas::signalRedraw()" src/codegen/builtins/canvas/func_present*.rs` → `func_present.rs`, `func_present_layers.rs` |
| canvas runtime test files | 15 | `ls tests/canvas/ \| wc -l` |
| hand-kept man-example member list | 1 | `grep -n "const MEMBERS" tests/cli/cli_canvas_man_examples_compile.rs` |

### Verified properties

- **`frameDone` already holds the mutex and broadcasts.** Read in
  `emit_frame_done`. The new frame record goes inside that same critical
  section, so it adds no lock.
- **A skipped frame reaches `frameDone`.** Read in the render loop and in
  the skip branch of `__canvas_renderFrame`. So "drew" has to be passed in;
  it can't be inferred from the frame counter.
- UNVERIFIED: whether the Metal "direct present" path (`directFrames=`) goes
  through the same `__canvas_renderFrame` return as a normal Metal frame.
  Phase 2 reads `__canvas_renderMetal` and tags it `Metal` either way.

## 3. Design Overview

**The public surface.**

```
ENUM Renderer            ' ordinal order is frozen once shipped
  Software
  Metal
  Vulkan
END ENUM

TYPE FrameStats
  dt AS Float            ' seconds between the last two presents; 0 before the second
  pps AS Float           ' presents in the trailing 1 s (see "rate" below)
  fps AS Float           ' frames drawn in the trailing 1 s
  displayHz AS Float     ' the window's monitor; 0 when unknown
  frameSeconds AS Float  ' render time of the last drawn frame; 0 before the first
  presents AS Integer    ' total since start
  frames AS Integer      ' total frames drawn since start
  renderer AS Renderer   ' what drew the last frame; Software before the first
END TYPE
```

**Storage.** Append to the graphics state, after plan-155-A's words (824 is
the size once A lands):

| Offset | Word | Writer |
|---|---|---|
| 824 | `PRESENTS` total | worker, under the mutex |
| 832 | `PRESENT_HEAD` ring index | worker |
| 840 | `PRESENT_RING`, 64 × 8 B timestamps | worker |
| 1352 | `DRAWN` total | graphics, in `frameDone` |
| 1360 | `DRAWN_HEAD` | graphics |
| 1368 | `DRAWN_RING`, 64 × 8 B | graphics |
| 1880 | `LAST_RENDER_NANOS` | graphics |
| 1888 | `LAST_RENDERER` ordinal | graphics |

`GRAPHICS_STATE_SIZE` becomes 1896. Both rings are written and read only
under `GRAPHICS_OFFSET_MUTEX`.

**The rate over a window**, one MFBASIC helper `__canvas_rate(ring, head,
total, now)`:

- `k` is the number of ring entries newer than `now - 1 s`, counting only
  entries that exist (`min(total, 64)` of them).
- If `k < 64`, the rate is `k`.
- If `k = 64`, the ring spans less than a second, so the rate is
  `63 / (newest - oldest)` in seconds.

That gives an exact count at ordinary rates, and a correct rate above 64/s
(for example a progress loop presenting thousands of times a second) without
an unbounded ring. It is 0 one second after the last event.

**Native versus MFBASIC.** There is one native internal
`canvas::frameCounters() AS List OF Integer`. When `STARTED` is 1 it takes the
mutex once and copies, in order:

1. `now`
2. `PRESENTS`
3. `PRESENT_HEAD`
4. `DRAWN`
5. `DRAWN_HEAD`
6. `LAST_RENDER_NANOS`
7. `LAST_RENDERER`
8. `displayMilliHz`
9. the 64 present timestamps
10. the 64 drawn timestamps

That is 136 Integers. When `STARTED` is 0 it returns the same shape without
locking: zeros, with `now` and `displayMilliHz` filled in.

`getFrameStats` is an MFBASIC body that builds `FrameStats` from that list.
**One lock means one moment**, so `pps - fps` is never a comparison across two
samples. The arithmetic stays in readable MFBASIC.

**Recording.**

- A new native internal `canvas::notePresent()` bumps `PRESENTS`, writes
  `frameNanos` into the ring, and advances the head, under the mutex.
- In both wrappers, `__canvas_ensureGraphics()` moves out of the `IF` to just
  before it. It is idempotent, and costs one Boolean test after the first
  call. `canvas::notePresent()` goes directly after it, unconditionally. So
  every present is counted, and the mutex always exists.
- `canvas::frameDone(drew AS Boolean, renderer AS Integer, renderNanos AS Integer)`
  gets these arguments. When `drew` is TRUE it also bumps `DRAWN`, writes the
  ring, and stores the nanos and the renderer. The existing
  `FRAMES`/broadcast behavior is unchanged for both values of `drew`.
- `__canvas_renderFrame` sets two graphics-thread globals,
  `__CANVAS_LAST_DREW` and `__CANVAS_LAST_RENDERER`. The render loop times
  `__canvas_renderFrame` with `canvas::frameNanos` and passes all three to
  `frameDone`.

**Where correctness risk concentrates:**

- **Moving `ensureGraphics` ahead of the `IF`.** A first present of a scene
  identical to the empty initial one now starts the graphics thread, where
  before it did not. That is harmless: it draws the scene that is installed.
  But it changes when the thread starts, so the full `rt_canvas_*` set is the
  check (Phase 1).
- **The `frameDone` signature change** touches the one call site in
  `render_loop_head!` (`helper_render.rs`). The debug and release variants
  share that text, so there is one edit.

**Gate class:** new behavior. There is no byte-identity gate. Every canvas
program's `.ncode` diffs, because `present` gains a call. The sentinel
`tests/syntax/app/app-mouse-surface` is expected to diff: inspect it, then
re-baseline those lines only.

**Rejected alternatives:**

- **Counters as MFBASIC globals.** They are per-thread (§2), so the graphics
  thread's frame count would be invisible to the worker.
- **An average updated per event** (for example an exponential moving
  average of the interval). It never decays: a static scene would show its
  last rate forever.
- **Separate `getDeltaTime`, `getPps` and `getFps` functions.** They sample
  at different moments, and the discussion chose one record.
- **Integer milliseconds for `dt`.** At 120 Hz that flips between 8 and 9,
  a 12% error on every frame.
- **Counting skipped wake-ups as frames.** They drew nothing, so they are
  not "the rate the backend draws at".

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; strike moot tasks with evidence; fill
> `Commit:` the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — storage and recording (internal only)

- [ ] `runtime/canvas/mod.rs`: add the §3 offsets and set
      `GRAPHICS_STATE_SIZE` to 1896. Add `emit_note_present`, extend
      `emit_frame_done` with the `drew` / renderer / nanos arguments, and add
      `emit_frame_counters` (with the `STARTED` check).
- [ ] `func_graphics.rs`: register internal `notePresent`, the new
      `frameDone` parameters, and `frameCounters`.
- [ ] `func_present.rs` and `func_present_layers.rs`: move `ensureGraphics`
      ahead of the `IF` and add `canvas::notePresent()`.
- [ ] `helper_render.rs`: set `__CANVAS_LAST_DREW` and
      `__CANVAS_LAST_RENDERER` on every return path of
      `__canvas_renderFrame` (Metal, Vulkan, software, skip), time the call,
      and pass all three to `frameDone`.
- [ ] Check that the move changed nothing covered: run the full canvas
      runtime set, because the change touches every present.

Acceptance: every existing canvas runtime test still passes.
  Check: `cargo test --test 'rt_canvas_*'` → all pass (est. 12 min; the moved
  `ensureGraphics` changes when the thread starts on every present path, and
  no single file covers all of them).
Commit: —

### Phase 2 — the public record and function

- [ ] `canvas/mod.rs`: register `Renderer` (Software, Metal, Vulkan) and
      `FrameStats`, with `description` prose per field.
- [ ] Add `func_get_frame_stats.rs`, whose MFBASIC body builds the record from
      `frameCounters()` through `__canvas_rate`. Register it in
      `mod.rs:register`.
- [ ] Read `__canvas_renderMetal` for the direct-present path (§2
      UNVERIFIED) and record the answer in Corrections.
- [ ] Tests, in a new `tests/canvas/rt_canvas_frame_stats.rs` (a `[[test]]`
      entry in `Cargo.toml` beside `rt_canvas_graphics_thread`), headless and
      cross-platform:
      - `before_any_present_every_field_is_zero`: every field is 0 and
        `renderer` is `Software`, except `displayHz`, which equals
        `MFB_CANVAS_DISPLAY_HZ=90`.
      - `identical_presents_count_as_presents_not_frames`: under
        `MFB_CANVAS_SYNC`, 5 changing presents then 5 identical ones give
        `presents=10` and `frames=5`.
      - `dt_is_the_gap_between_the_last_two_presents`: two presents 50 ms
        apart (`os::sleep(50)`) give `dt >= 0.05`, and `dt = 0` after exactly
        one present.
      - `rates_fall_to_zero_after_a_second_idle`: 20 presents, sleep 1200 ms,
        then `pps = 0` and `fps = 0`.
      - `pps_above_the_ring_is_measured_not_capped`: 500 presents in a tight
        loop give `pps > 64`.
      - `frame_seconds_and_renderer_after_a_drawn_frame`: under SYNC,
        `frameSeconds > 0` and `renderer = Software`. With `MFB_CANVAS_GPU=1`
        on macOS, `Metal`, gated like the existing Metal cases.
- [ ] Man page: `intro`, `desc` and `example` on `getFrameStats`, following
      `.ai/man-content.md`. The desc says plainly that a static scene reports
      `fps = 0`, and that `fps` below `pps` means presents are arriving faster
      than frames are drawn. Add `getFrameStats` to
      `tests/cli/cli_canvas_man_examples_compile.rs:MEMBERS`.
- [ ] Spec, `src/docs/spec/app/06_canvas.md` "Rendering conventions": what
      each field counts, the one-second window, and the 64-entry saturation
      rule, cited to `[[src/codegen/runtime/canvas/mod.rs:emit_frame_counters]]`
      and the `__canvas_rate` helper.
- [ ] `.ai/canvas-threading.md`: a new §15 on frame statistics (the two rings,
      single-lock snapshot, why they are not MFBASIC globals).

Acceptance: the six tests pass; the man page renders and its example runs.
  Check: `cargo test --test rt_canvas_frame_stats` → 6 passed (est. 4 min);
  `scripts/man-run-examples.sh canvas --run` → all pass (est. 3 min);
  `cargo test --bin mfb spec` → pass, and `scripts/spec-census.sh --citations`
  → `MISS-SYMBOL 0` (est. 3 min).
Commit: —

## Validation Plan

- Tests: `rt_canvas_frame_stats.rs` (six cases, including the idle decay and
  saturation edges); the whole `rt_canvas_*` set after Phase 1.
- Coverage check: `grep -rn "frameCounters\|notePresent" src tests` → the
  emitters, both present wrappers, the render loop, the public body, and the
  tests.
- Runtime proof: wind printing `getFrameStats()` for a few seconds is
  plan-155-C's proof.
- Doc sync: the man page, the spec's "Rendering conventions",
  `.ai/canvas-threading.md` §15.
- Final gate: once, at the end of plan-155-C.

## Open Decisions

- **`renderer` before the first frame: `Software` with `frames = 0`**
  (recommended; no variant exists only to mean "not yet") **vs a leading
  `None` variant.** Adding one later would renumber the enum, so decide
  before Phase 2.
- **Window length: a fixed 1 s** (recommended; one number, easy to explain)
  **vs a parameter.** It can become a defaulted parameter later without
  breaking callers.

## Corrections

(none yet)

## Summary

The risk is in Phase 1: `ensureGraphics` moves to every present, and
`frameDone` gains arguments. Both are checked against the whole existing
canvas runtime set before any public surface is added. Nothing about
rendering, the frame skip, the redraw triggers, or `present`'s non-blocking
contract changes.
