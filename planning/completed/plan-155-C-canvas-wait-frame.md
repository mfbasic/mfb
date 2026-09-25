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
| plan-155-B complete | `ls planning/plan-155-B-*` → no match (archived) | MET (2026-09-24: `no matches found`) |
| `getFrameStats` is public | `mfb man canvas getFrameStats` → a page | MET (2026-09-24: `GETFRAMESTATS` page) |

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

- [x] Read how `emit_thread_external_call` maps pthread calls on Windows, and
      pick the timed wait (§2 UNVERIFIED). Record it in Corrections. → it maps
      `pthread_cond_timedwait` (absolute deadline) and `clock_gettime` onto
      `SleepConditionVariableSRW` / `GetSystemTimePreciseAsFileTime`; the floor uses
      them unchanged on every family (Corrections 1).
- [x] `runtime/canvas/mod.rs`: add `FRAME_COND` and `LAST_WAIT` (size 1952),
      initialize `FRAME_COND` in `emit_start_graphics`, broadcast it in
      `emit_wait_for_redraw` after clearing `PENDING` and on the stop path,
      and add `emit_await_pickup`. → 1896/1944, size 1952 as planned; imports for
      the timed wait and the clock added to each platform's canvas arm.
- [x] `func_graphics.rs`: register internal `awaitPickup(intervalNanos AS Integer)`.
      Also internal `requireCanvasMode()`, which carries the mode gate (Corrections 2).
- [x] Add `func_wait_frame.rs`, the public MFBASIC wrapper with the mode gate,
      registered in `mod.rs:register`.
- [x] Tests, in `tests/canvas/rt_canvas_frame_stats.rs` (headless, all
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
      → this Mac: `cargo test --test rt_canvas_frame_stats wait_frame` → 5 passed;
      `cargo test --test rt_canvas_graphics_thread static_scene` → 2 passed (the new
      `a_static_scene_under_a_wait_frame_loop_renders_once` and the existing one).
      On macOS R25 proves only that the loop finishes: the scripted resize exists on
      GTK and Windows only (Corrections 3).

Acceptance: the six cases pass on this Mac, plus the pacing and pickup cases
on Linux and Windows. Verified: Mac 5 + 2 passed; box 2226 (in place of 2223,
Corrections 4) and box 2230 — every bound held (below).
  Check: `cargo test --test rt_canvas_frame_stats wait_frame` → 5 passed, and
  `cargo test --test rt_canvas_graphics_thread static_scene` → pass (est. 5 min).
  The same filter on box 2223 (native aarch64 Linux, est. 8 min) and box 2230
  (Windows, est. 10 min), because the timed wait is platform-mapped code that
  macOS doesn't exercise.
  Box results — the five programs cross-built with `--app --debug` and run headless
  with each test's environment (`/tmp/p155/cbox`):
  - 2226 linux-aarch64 glibc: changing `loopMs=821` (≥ 480), unchanged `849`, first
    `returned`, pickup `812` (≥ 400), resize `finished` with 37 frames at
    `damage=0,0,640,480`. Rerun alone, unchanged gives 1063/555/293/27 ms at
    25/50/100/1000 Hz — the interval plus ~1 ms; the 849 was the first run of a batch
    that was also extracting AppImages.
  - 2230 windows-x86_64: changing `1988`, unchanged `836`, first `returned`, pickup
    `926`, resize `finished` and a frame at 640x480. The unchanged loop's ~33 ms per
    20 ms interval is the default ~15.6 ms Windows timer tick rounding
    `SleepConditionVariableSRW`'s timeout up — a floor, never early.
Commit: `9b90a502b`

### Phase 2 — docs and wind

- [x] Man page for `waitFrame`: `intro`, `desc` and `example` (the loop in
      this plan's header), following `.ai/man-content.md`. Add it to
      `cli_canvas_man_examples_compile.rs:MEMBERS`. → the example is a bounded
      60-iteration version of the header loop (a page example must end);
      `cargo test --test cli_canvas_man_examples_compile` → 1 passed.
- [x] `present`'s man `desc` (`func_present.rs:DESC`) and `MODULE_DESC`: say
      that `present` returns immediately, and point animation loops at
      `waitFrame` + `getFrameStats`.
- [x] Spec, `06_canvas.md`: `waitFrame`'s two conditions, the floor, and why
      it is "picked up", not "drawn", cited to `emit_await_pickup`.
- [x] `.ai/canvas-threading.md`:
      - §8, rows R22–R25:
        - R22 `waitFrame` while the graphics thread stops;
        - R23 before the first present;
        - R24 pickup with a held frame;
        - R25 resize during the wait.
      - §9, one line: `waitFrame` waits and never repaints.
      - §15 (from plan-155-B): the second condition variable and why it is
        separate.
- [x] `examples/wind/src/main.mfb`: remove `FRAME_MILLIS`, `lastNanos`,
      `startNanos`, `spentMillis` and the sleep. Use
      `math::min(canvas::getFrameStats().dt, MAX_STEP_SECONDS)` and end the
      loop body with `canvas::waitFrame()`. Rewrite the comment above
      `FRAME_MILLIS` to explain the new loop. Drop `IMPORT datetime` and
      `IMPORT os` if nothing else uses them. → `IMPORT datetime` dropped; `IMPORT os`
      stays (`os::name()`); `mfb build --app examples/wind` → success.
- [x] Runtime proof: run wind windowed on this Mac for 15 s with
      `MFB_CANVAS_STATS` in a `--debug` build, and log `getFrameStats()` once
      a second to stderr in a scratch copy (not committed). Record `pps`,
      `fps` and `displayHz`: `pps` should sit near `displayHz`, where before
      it was capped near 62 by the 16 ms sleep. Record the numbers, not the
      prediction. → measured, windowed, `displayHz=120.00` throughout (logged with
      `io::print`, which has no stderr twin — stdout serves):
      - `--debug` + `MFB_CANVAS_STATS`: `pps` 18-19, `fps` 18, 357 stats lines, all
        `directFrames` (a real window).
      - release: `pps=26.00 fps=26.00`; `waitFrame` waited 47 ms in total over 420
        presents.
      - the old loop, built by the pre-plan compiler and timed the same way: ~34 ms of
        work a frame, 420 frames in 15 s (~28 fps) — it was never near 62.
      Wind is work-bound on this machine (its step + scene take ~35-38 ms), so its `pps`
      cannot reach 120; the prediction was wrong (Corrections 7). The display-pace claim
      is shown by a light windowed loop instead (`/tmp/p155/pace`, the example's shape,
      5 s): `pps=120.03 fps=119.96`, `120.00/119.99`, `119.98/120.07` at
      `displayHz=120.00`.
- [x] (added) The floor's cadence (Corrections 6): that light loop first measured
      `pps=99-100` at 120 Hz. `LAST_WAIT` now takes the deadline when a return is less
      than an interval late. Re-proved: this Mac 120.0; 2226 unchanged-scene loops
      1004/503/251/25 ms at 25/50/100/1000 Hz; the five box programs re-run on 2226
      (689/689/returned/723/finished, 37 frames at 640x480) and 2230
      (2018/601/returned/940/finished, frame at 640x480); `rt_canvas_frame_stats` 11
      passed and `static_scene` 2 passed locally.

Acceptance: docs render and examples run; wind builds, runs, and its measured
`pps` tracks `displayHz`. **Corrected** (Corrections 7): wind's `pps` is bound by its
own ~35-38 ms of work, so the checkable form is — a light loop measures `pps` within
1% of `displayHz` in a real window, and wind's `waitFrame` time is under 1% of its loop
(47 ms of ~16.5 s).
  Check: `scripts/man-run-examples.sh canvas --run` → all pass (est. 3 min);
  `cargo test --bin mfb spec` → pass (est. 2 min);
  `mfb build examples/wind` → success (est. 1 min); the runtime proof above
  (est. 3 min, needs a real window).
  Verified: examples 31 built / 31 ran / 0 failed; spec 44 passed, `MISS-SYMBOL 0`,
  `man-census --memory-scope` 0 unclassified; wind builds; runtime as recorded above.
Commit: `1367cfa57`

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

### Final gate (run once, here — narrowed by the user to the canvas tests)

- Before merging main: `cargo test --test 'rt_canvas_*' --test 'cli_canvas_*'` → 19
  suites, 229 passed, 0 failed, 4 ignored.
- Main had advanced 77 files (bug-680/689/697, wind's swarm). Overlap with this plan:
  `Cargo.toml` (both added `[[test]]` entries — merged cleanly) and the four canvas
  sentinel `.app.ncodesum` (conflicted; regenerated from the merged compiler — every
  other app golden already matched it). No source file overlapped, so per the user the
  gate after the merge is the canvas set again: 19 suites, 229 passed, 0 failed,
  4 ignored. App acceptance passed; merged wind builds.
- `cargo fmt --all` over both workspaces: whitespace only, in this plan's files.

## Open Decisions

- **Floor when the rate is unknown: 60 Hz** (recommended; the most common
  display) **vs no floor.** No floor spins a core headless, so it is not
  really an option.
- **Should `waitFrame` report how long it waited?** Recommended **no**:
  `getFrameStats().dt` already gives the loop period, and returning a value
  now would be hard to remove later.

## Corrections

1. **The timed wait (§2 UNVERIFIED).** `emit_windows_thread_call` realizes
   `pthread_cond_timedwait` with an absolute `timespec` against the clock its
   `clock_gettime` arm reads (`GetSystemTimePreciseAsFileTime`), and POSIX's default
   condition clock is `CLOCK_REALTIME`. So `awaitPickup` keeps its deadline and
   `LAST_WAIT` in `CLOCK_REALTIME` nanoseconds on every family — no second, monotonic
   clock in native code. A wall-clock step is bounded by never letting the deadline
   exceed `now + interval`.
2. **The mode gate is its own member.** `waitFrame` must raise `ErrWrongMode` before
   `__canvas_ensureGraphics` starts a render thread, and an MFBASIC body cannot carry
   `prepend_wrong_mode_gate`. It cannot go on `displayMilliHz` either: the stats line
   calls that on the graphics thread, whose presentation-mode word (per-thread arena
   state) is zero. So an internal `canvas::requireCanvasMode()` is the gate alone, and the
   wrapper calls it first.
3. **R25 on macOS.** `MFB_CANVAS_RESIZE_W`/`_H` is implemented by the GTK and Windows
   backends only (`grep -rln MFB_CANVAS_RESIZE_W src/target` → `linux_gtk`, `win_x86_64`),
   so on this Mac the test proves the loop finishes and asserts the resized frame only
   off macOS. The resized frame was observed on 2226 and 2230.
4. **Box 2223 was unreachable** (`ssh -p 2223` → connection refused), so the Linux row ran
   on 2226 (aarch64 glibc, the same architecture). Neither 2226 nor 2230 runs the cargo
   suite, so the rows ran the five test programs cross-built here, under each test's
   environment and bound — the programs and bounds are the tests', not approximations.
5. **`DIV` is Float division.** The wrapper first computed `1000000000000 DIV mhz` and
   failed to compile (`Assignment to interval has type Float`); `/` on two Integers is
   the integer division (`mfb spec language operators`: "`DIV` always returns `Float`").
6. **The floor lost its cadence.** §3 said "Store `LAST_WAIT = now` (not `deadline`: a
   loop that fell behind does not get a burst of catch-up returns)". Measured in a real
   window, a loop doing almost nothing ran at 99-100 presents a second on a 120 Hz
   display: every return is ~1.7 ms past its deadline, and restarting each interval from
   the return adds that to every frame. The rule is now the deadline when the return is
   less than one interval late, and `now` otherwise — cadence kept, and still no burst
   (a loop an interval or more behind restarts from now). Re-measured: 120.0.
7. **Wind cannot show `pps` tracking `displayHz`.** The acceptance assumed wind's loop
   is faster than a display interval. Measured, it is not: its step and scene take ~35-38
   ms (release) on this Mac, and the old loop was already at ~28 fps, never at the 62
   its sleep would allow. `waitFrame` waited 47 ms in 16.5 s of wind, so it is not
   what bounds it. The criterion is replaced by the checkable pair in the Phase 2
   acceptance — a light loop at the display rate, and wind's wait share — rather than
   dropped.
8. **Final gate narrowed by the user** (plan-155-A Correction 5): the canvas test set,
   not `cargo test`.

## Summary

The engineering risk is wake-up discipline: a second condition variable and
two broadcast sites. The four deadlock rows (R22–R25) pin it. `present`,
the frame skip, the redraw triggers and every example other than wind are
untouched. `waitFrame` only waits; it never causes a frame.
