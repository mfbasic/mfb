# plan-155-C: `canvas::waitFrame()`, and wind adopts it

Last updated: 2026-09-24
Effort: medium (1h–2h)
Depends on: plan-155-B

Add `canvas::waitFrame()`, which paces an animation loop:

```
DO WHILE running
  LET s AS canvas::FrameStats = canvas::getFrameStats()
  step(math::min(s.dt, MAX_STEP))
  canvas::present(scene())
  canvas::waitFrame()
LOOP
```

It returns once **both** of these hold:

1. **The renderer has picked up the last redraw the program asked for.** The
   program then builds its next scene while that frame draws, so one frame is
   in flight.
2. **At least one display interval has passed since `waitFrame` last
   returned.** The interval is `1 / displayHz`, or 1/60 s when the rate is
   unknown.

Condition 2 matters on every backend. The platform survey for plan-155-A found
that no backend waits for the display: Vulkan renders offscreen and reads
back, and the software paths post a blit to the UI thread. Without the floor,
an unchanged scene (nothing is owed) or a cheap one would spin a core.

Then `examples/wind` drops `FRAME_MILLIS`, its hand-timed `os::sleep` and its
own clock reads, and uses `getFrameStats().dt` plus `waitFrame`.

References:

- plan-155-A (the display rate) and plan-155-B (`getFrameStats`, and
  `ensureGraphics` on every present).
- `.ai/canvas-threading.md` — §3 (`present` ordering), §4 (redraw triggers),
  §8 (the race matrix, which this plan extends), §9 (no time-based repaint,
  still true: `waitFrame` waits, it never causes a frame).
- `.ai/man-content.md`.

## Prerequisites

See plan-155-A for the feature-wide prerequisites.

| Must be true | Command | Status |
|---|---|---|
| plan-155-B complete | `ls planning/plan-155-B-*` → no match (archived) | NOT MET |
| `getFrameStats` is public | `mfb man canvas getFrameStats` → a page | NOT MET |

If plan-155-B is not complete, this plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue, and again before you decide to stop.

## 1. Goal

- `canvas::waitFrame()` is public and documented, and it returns under
  exactly the two conditions above.
- A loop of `present` + `waitFrame` runs at the display rate, whether its
  scene changes or not, on every backend.
- `waitFrame` never deadlocks:
  - before the first present;
  - while a resize is being handled;
  - when the graphics thread stops.
- wind runs on the new loop, and its three pacing mechanisms are removed.

### Non-goals (explicit constraints)

- **`present` does not change.** It stays non-blocking (R5 and R6). Folding
  the wait into `present` was rejected in discussion; see §3.
- **No time-based repaint.** `waitFrame` never signals a redraw; §4 and §9
  hold.
- **The render loop's wait is not weakened.** Its own wake-ups must still
  arrive (see the condition variable below).

## 2. Current State

**The wait already exists as a test switch.**
`runtime/canvas/mod.rs:emit_sync_frame` (`canvas::syncFrame`) waits on
`GRAPHICS_OFFSET_COND` until `FRAMES >= WANTED`. It is a no-op unless
`MFB_CANVAS_SYNC` set `GRAPHICS_OFFSET_SYNC`.

**The pending flag is the "picked up" signal.**

- `emit_signal_redraw` sets `GRAPHICS_OFFSET_PENDING` (and `WANTED`) under the
  mutex, then calls `pthread_cond_signal`.
- `emit_wait_for_redraw` takes it: it waits while it is clear, then clears it.

Every trigger goes through `signalRedraw`: a present, a group change,
`setBytes`, and a resize on macOS and GTK. So "`PENDING` is clear" means
exactly "the renderer has picked up every redraw asked for so far".

**The shared-condition-variable hazard.** The render loop, `syncFrame` and
`frameDone` all use one `GRAPHICS_OFFSET_COND`:

- `signalRedraw` uses `pthread_cond_signal`, which wakes **one** waiter.
- The stop path (`emit_stop_graphics`, the `STOPPING` store) also uses
  `pthread_cond_signal`.

If the worker is blocked in a new wait on that same condition variable, a
main-thread resize signal can wake the worker instead of the render loop. The
redraw is then owed but never taken, and the worker waits for a pickup that
never happens. `syncFrame` has this hazard today, but only under the test
switch.

**The stop path.** `STOPPING` is set under the mutex and followed by a
`pthread_cond_signal`. The render loop's `waitForRedraw` returns FALSE on it.

**Wind today** (`examples/wind/src/main.mfb`, `main`):

- `FRAME_MILLIS = 16`;
- two `datetime::monotonicNanos` reads per frame;
- `elapsed` clamped to `MAX_STEP_SECONDS`;
- `IF spentMillis < FRAME_MILLIS THEN os::sleep(FRAME_MILLIS - spentMillis)`.

### Measured populations

| What | Count | Command |
|---|---|---|
| example programs that call `canvas::present` | 4 | `grep -rl "canvas::present" examples \| grep '\.mfb$' \| wc -l` |
| of those, pacing with `os::sleep` | 1 (wind) | `grep -rn "os::sleep" examples/wind examples/dungeon examples/emoji examples/bugs/canvas` → only `wind/src/main.mfb:334` |
| users of `GRAPHICS_OFFSET_COND` waits | 2 (`waitForRedraw`, `syncFrame`) | `grep -n "pthread_cond_wait" src/codegen/runtime/canvas/mod.rs` |

### Verified properties

- **Every redraw trigger sets `PENDING` through `signalRedraw`.** Read in
  `func_present.rs`, `func_present_layers.rs`, `func_set_bytes.rs`, and the
  macOS/GTK resize helpers (plan-155-A §2). Windows `WM_SIZE` signals nothing:
  plan-155-A Phase 1 settles whether that is a bug. Either way it is not a
  hang for `waitFrame`, which only waits on `PENDING` being set.
- UNVERIFIED: **whether `pthread_cond_timedwait` and `clock_gettime` are
  available through `emit_thread_external_call` on Windows**, which may map
  pthreads onto Win32. Phase 1's first task reads that mapping. The floor's
  sleep uses whatever timed wait it offers, and the choice is recorded in
  Corrections.

## 3. Design Overview

**A second condition variable.** Append `GRAPHICS_OFFSET_FRAME_COND` (48
bytes, like `COND`) after plan-155-B's words. `GRAPHICS_STATE_SIZE` goes from
1896 to 1944. It is initialized beside `COND` in `emit_start_graphics`.

- `emit_wait_for_redraw` broadcasts `FRAME_COND` right after it clears
  `PENDING`, while it still holds the mutex.
- The stop path broadcasts it too.
- `waitFrame` waits **only** on `FRAME_COND`, so it can never take a wake-up
  meant for the render loop.

**The last return time** (`GRAPHICS_OFFSET_LAST_WAIT`, 8 bytes; the state
becomes 1952) is written only by `waitFrame`.

**`canvas::waitFrame()`** is an MFBASIC wrapper:

1. Call `__canvas_ensureGraphics()`. The mutex and condition variables then
   exist even before the first present.
2. Call a native internal `canvas::awaitPickup(intervalNanos)`, where
   `intervalNanos = 1e12 / displayMilliHz()`, or 16,666,667 when that is 0.

`awaitPickup`, under the mutex:

- `deadline = LAST_WAIT + interval`.
- Loop:
  - return if `STOPPING`;
  - otherwise, if `PENDING` is clear and `now >= deadline`, leave the loop;
  - otherwise, if `PENDING` is set, wait on `FRAME_COND`;
  - otherwise, timed-wait on `FRAME_COND` until the deadline.
- Store `LAST_WAIT = now` (not `deadline`: a loop that fell behind does not
  get a burst of catch-up returns).

**Outside `Mode.Canvas`**, it raises the error `present` raises there. It
copies the mode gate `gen_present.rs:emit_publish` prepends.

**Why "picked up" and not "drawn".** Waiting until the frame has *finished*
serializes the two threads: the worker would idle while the frame renders,
and the renderer would idle while the worker builds. "Picked up" leaves one
frame in flight. Wind's build and render times then overlap instead of
adding.

**Where correctness risk concentrates:**

- **Wake-up discipline.** A missed broadcast is a hang. Every path that
  clears `PENDING` or sets `STOPPING` must broadcast `FRAME_COND`. There are
  two, and both are tested (R24, R25).
- **`MFB_CANVAS_SYNC` composes.** Under it, `present` has already waited for
  the frame, so `PENDING` is clear, `waitFrame` applies the floor only, and
  the existing sync tests are unaffected.

**Gate class:** new behavior. Wind's `.ncode` is expected to diff, since that
program changes. No other example changes. Existing canvas tests must stay
green; in particular `a_static_scene_renders_once_and_does_not_spin` proves
`waitFrame` added no repaint.

**Rejected alternatives:**

- **`present` waits** (always, or with `wait := TRUE`). It breaks R5 and R6,
  throttles programs that present progress while computing, and makes an
  identical re-present sleep. It was discussed and rejected.
- **Reusing `COND` with `pthread_cond_broadcast` everywhere.** It fixes the
  lost wake-up, but it wakes the render loop on every `waitFrame`-side event
  and the reverse, which is a thundering herd on the frame path. A second
  condition variable costs 48 bytes.
- **Waiting for `FRAMES >= WANTED`** (what `syncFrame` does). That waits for
  the frame to *finish*, which serializes the threads (see above).
- **A display-link callback (CVDisplayLink / GTK frame clock / DWM).** That is
  real vsync, but it is three new platform mechanisms. The floor gives the
  same pacing from one number plan-155-A already publishes. If a measured
  program shows the floor's jitter matters, add a display link then.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; strike moot tasks with evidence; fill
> `Commit:` the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — `waitFrame`

- [ ] Read how `emit_thread_external_call` maps pthread calls on Windows, and
      pick the timed wait (§2 UNVERIFIED). Record it in Corrections.
- [ ] `runtime/canvas/mod.rs`: add `FRAME_COND` and `LAST_WAIT` (size 1952),
      initialize `FRAME_COND` in `emit_start_graphics`, broadcast it in
      `emit_wait_for_redraw` after clearing `PENDING` and on the stop path,
      and add `emit_await_pickup`.
- [ ] `func_graphics.rs`: register internal `awaitPickup(intervalNanos AS Integer)`.
- [ ] Add `func_wait_frame.rs`, the public MFBASIC wrapper with the mode gate,
      registered in `mod.rs:register`.
- [ ] Tests, in `tests/canvas/rt_canvas_frame_stats.rs` (headless, all
      platforms):
      - `wait_frame_paces_a_changing_scene`: `MFB_CANVAS_DISPLAY_HZ=50`,
        25 × (changing present; waitFrame) takes ≥ 480 ms.
      - `wait_frame_paces_an_unchanged_scene`: the same with an identical
        scene takes ≥ 480 ms. This is the spin guard.
      - `wait_frame_before_any_present_returns` (R23): one call returns and
        the program exits 0.
      - `wait_frame_waits_for_pickup` (R24): with
        `MFB_CANVAS_FRAME_HOLD_MS=100` and `MFB_CANVAS_DISPLAY_HZ=1000`,
        6 changing presents with waitFrame take ≥ 400 ms. That is paced by
        the renderer, not the floor.
      - `wait_frame_survives_a_resize` (R25): `MFB_CANVAS_RESIZE_W`/`_H`
        fires during a present + waitFrame loop, the loop finishes, and the
        stats line shows the resized frame.
      - `a_static_scene_renders_once_and_does_not_spin` (existing) is
        re-run with a waitFrame loop variant added beside it: still 1 frame.

Acceptance: the six cases pass on this Mac, plus the pacing and pickup cases
on Linux and Windows.
  Check: `cargo test --test rt_canvas_frame_stats wait_frame` → 5 passed, and
  `cargo test --test rt_canvas_graphics_thread static_scene` → pass (est. 5 min).
  The same filter on box 2223 (native aarch64 Linux, est. 8 min) and box 2230
  (Windows, est. 10 min), because the timed wait is platform-mapped code that
  macOS doesn't exercise.
Commit: —

### Phase 2 — docs and wind

- [ ] Man page for `waitFrame`: `intro`, `desc` and `example` (the loop in
      this plan's header), following `.ai/man-content.md`. Add it to
      `cli_canvas_man_examples_compile.rs:MEMBERS`.
- [ ] `present`'s man `desc` (`func_present.rs:DESC`) and `MODULE_DESC`: say
      that `present` returns immediately, and point animation loops at
      `waitFrame` + `getFrameStats`.
- [ ] Spec, `06_canvas.md`: `waitFrame`'s two conditions, the floor, and why
      it is "picked up", not "drawn", cited to `emit_await_pickup`.
- [ ] `.ai/canvas-threading.md`:
      - §8, rows R22–R25:
        - R22 `waitFrame` while the graphics thread stops;
        - R23 before the first present;
        - R24 pickup with a held frame;
        - R25 resize during the wait.
      - §9, one line: `waitFrame` waits and never repaints.
      - §15 (from plan-155-B): the second condition variable and why it is
        separate.
- [ ] `examples/wind/src/main.mfb`: remove `FRAME_MILLIS`, `lastNanos`,
      `startNanos`, `spentMillis` and the sleep. Use
      `math::min(canvas::getFrameStats().dt, MAX_STEP_SECONDS)` and end the
      loop body with `canvas::waitFrame()`. Rewrite the comment above
      `FRAME_MILLIS` to explain the new loop. Drop `IMPORT datetime` and
      `IMPORT os` if nothing else uses them.
- [ ] Runtime proof: run wind windowed on this Mac for 15 s with
      `MFB_CANVAS_STATS` in a `--debug` build, and log `getFrameStats()` once
      a second to stderr in a scratch copy (not committed). Record `pps`,
      `fps` and `displayHz`: `pps` should sit near `displayHz`, where before
      it was capped near 62 by the 16 ms sleep. Record the numbers, not the
      prediction.

Acceptance: docs render and examples run; wind builds, runs, and its measured
`pps` tracks `displayHz`.
  Check: `scripts/man-run-examples.sh canvas --run` → all pass (est. 3 min);
  `cargo test --bin mfb spec` → pass (est. 2 min);
  `mfb build examples/wind` → success (est. 1 min); the runtime proof above
  (est. 3 min, needs a real window).
Commit: —

## Validation Plan

- Tests: the plan-155-B stats cases, the six `wait_frame` cases, and the
  static-scene guard.
- Coverage check: `grep -rn "awaitPickup\|FRAME_COND" src tests` → the
  emitter, the stop path, the redraw wait, the wrapper, and the tests.
- Runtime proof: wind's measured `pps`/`fps`/`displayHz` (Phase 2).
- Doc sync: man pages for `waitFrame`, `present` and the package intro; spec
  `06_canvas.md`; `.ai/canvas-threading.md` §8, §9 and §15.
- **Final gate for all of plan-155**, run once here:
  - `cargo test` (est. 45 min; it is the whole-suite gate);
  - `scripts/man-examples-gate.sh` (est. 10 min);
  - `scripts/spec-census.sh --citations` → `MISS-SYMBOL 0`;
  - the canvas runtime set on box 2223 and box 2230.

## Open Decisions

- **Floor when the rate is unknown: 60 Hz** (recommended; the most common
  display) **vs no floor.** No floor spins a core headless, so it is not
  really an option.
- **Should `waitFrame` report how long it waited?** Recommended **no**:
  `getFrameStats().dt` already gives the loop period, and returning a value
  now would be hard to remove later.

## Corrections

(none yet)

## Summary

The engineering risk is wake-up discipline: a second condition variable and
two broadcast sites. The four deadlock rows (R22–R25) pin it. `present`,
the frame skip, the redraw triggers and every example other than wind are
untouched. `waitFrame` only waits; it never causes a frame.
