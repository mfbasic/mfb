# plan-172-J: The main thread publishes the pointer to the graphics state

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-172-I

`AutoPaint` (172-K) is resolved on the graphics thread, so the graphics thread has to
know where the pointer is without the worker's help. This letter makes each platform's
main thread publish the pointer position, the button state and whether the pointer is
inside the surface into the graphics-state block, the way it publishes a resize.

Behavioral outcome: with the program blocked in `io::input`, moving the mouse over a
canvas window updates the pointer words the graphics thread reads, and a pointer that
leaves the surface is reported as outside — on macOS, GTK and Windows.

References:

- `planning/new_canvas.md` — "Where AutoPaint is resolved: the graphics thread".
- `.ai/canvas-threading.md` §1 (what each thread owns), §5 (the resize handshake, the
  precedent), §11 (test affordances; `MFB_CANVAS_RESIZE_W` / `_H`).
- `src/codegen/runtime/canvas/mod.rs` — `GRAPHICS_OFFSET_*`, `GRAPHICS_STATE_SIZE`,
  `emit_publish_surface_size`, `graphics_state_data_object`.
- `src/target/macos_aarch64/app/mouse_view.rs`, `src/target/linux_gtk/mouse.rs`,
  `src/target/win_x86_64/app/mouse.rs` — the three mouse producers.
- `.ai/arch-abi.md` — macOS AArch64, Win64 and GTK calling traps for the producers.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-I complete and archived | `ls planning/plan-172-I-*` → no matches | NOT MET |
| The Windows box is reachable | `ssh test@192.168.1.206 echo ok` → `ok` | UNVERIFIED — run first |
| Box 2226 has its GNOME Wayland session | probe per `.ai/remote_systems.md` | UNVERIFIED — run first |

If plan-172-I is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- The graphics-state block carries the pointer: `x`, `y` in surface pixels, a button
  mask, an inside flag, and a sequence word.
- All three platforms write them from the main thread on every pointer move, button
  change, and on the pointer entering or leaving the surface.
- They are written whether or not the program called `canvas::enableMouse`.
- The graphics thread reads them consistently, and a `--debug` build's stats line
  reports them as `pointer=`.
- A headless run can script the pointer.

### Non-goals (explicit constraints)

- **No redraw is signalled by a pointer change in this letter.** 172-K adds the trigger,
  conditional on the live scene holding an `AutoPaint`. Until then
  `.ai/canvas-threading.md` §4's five triggers stand.
- **The SGR pipe, the worker's ring and `canvas::pollMouse` are untouched.** Publishing
  is in addition to them.
- **The worker never reads or writes the pointer words.**
- **No backing-scale conversion is added.** The words carry the same surface pixels the
  SGR events carry today.

## 2. Current State

Each producer runs on the main thread and writes SGR-1006 text to the window input
pipe: macOS through eleven generated view methods including `mouseMoved:` and a tracking
area with options Moved, ActiveAlways and InVisibleRect; GTK through
`_mfb_gtkapp_mouse_{pressed,released,motion,scroll}`; Windows through eight `WM_*` arms
in the window procedure. Each range-checks the point against the published surface size
and drops an event outside it, so nothing is ever reported for the pointer leaving.

The resize path is the precedent for main-to-graphics state: plain 64-bit stores of
width and height into the block at fixed offsets
(`emit_publish_surface_size`), then `canvas::signalRedraw`. The block's size is derived
from its last offset, so appending words grows it without a second constant.

### Measured populations

| What | Count | Command |
|---|---|---|
| Mouse producer files | 3 (826, 673 and 715 lines) | `wc -l src/target/macos_aarch64/app/mouse_view.rs src/target/linux_gtk/mouse.rs src/target/win_x86_64/app/mouse.rs` |
| `canvas::signalRedraw` call sites in the targets | 10 | `grep -rho 'CANVAS_SIGNAL_REDRAW_SYMBOL' src/target \| wc -l` → 10 |
| Fixtures whose native goldens cover the producers | 1, on 4 targets | `ls tests/syntax/app/app-mouse-surface/golden/*.ncodesum \| wc -l` → 4 |
| Mouse-mode references across the three producers | 6 | `grep -n 'MOUSE_MODE' src/target/macos_aarch64/app/mouse_view.rs src/target/linux_gtk/mouse.rs src/target/win_x86_64/app/mouse.rs \| wc -l` → 6. Where each test sits relative to the coordinate is UNVERIFIED; read all six before placing a publish call |
| Whether the macOS tracking area is installed when the mouse is not enabled | UNMEASURED | read `mouse_view.rs:emit_install_tracking_area`'s caller |

### Verified properties

- **Motion with no button held is already produced on all three platforms.** VERIFIED by
  reading the three producers: each emits a Move with `SGR_NO_BUTTON`.
- **`MFB_CANVAS_RESIZE_W` / `_H` exists on GTK and Windows and not on macOS.** VERIFIED
  by searching `src/target/macos_aarch64` for the name. The scripted pointer below
  therefore cannot assume a native headless hook on macOS.
- **No test in the tree drives a real window's mouse.** VERIFIED:
  `canvas::pollMouse` appears under `tests/` only in a codegen fixture.

## 3. Design Overview

**The words.** Five 8-byte words appended after the block's current last offset:
`POINTER_SEQ`, `POINTER_X`, `POINTER_Y`, `POINTER_BUTTONS` (bit 0 left, 1 middle, 2
right), `POINTER_INSIDE`.

**One writer helper,** `emit_publish_pointer`, beside `emit_publish_surface_size`:
increment the sequence to odd, store the four words, increment to even. The graphics
thread reads the sequence, the words, and the sequence again, and retries on a change or
an odd value — the scene's sequence-lock pattern (`.ai/canvas-threading.md` §3), with
the same acquire/release instructions on AArch64.

**Each producer calls it** at the point it already has the surface-pixel coordinate,
before its mouse-mode test, so publication does not depend on `enableMouse`.

**Leaving the surface** is new on each platform: macOS adds entered-and-exited to the
tracking area options and a `mouseExited:` method; GTK connects the motion controller's
`leave` signal; Windows calls `TrackMouseEvent` on the first move and handles
`WM_MOUSELEAVE`. Each publishes `INSIDE = 0`. A move outside the surface bounds also
publishes `INSIDE = 0` rather than being dropped.

**The graphics-thread reader** is one internal native, `canvas::pointerSnapshot`,
returning the four values. The stats line prints them.

**Scripting, two layers:**

- **A debug-build script on the graphics thread**, for the state tests 172-K needs on
  every platform including macOS. `MFB_CANVAS_POINTER` holds
  `ms,x,y,buttons,inside;…`; at each frame start, entries whose time has passed are
  applied through `canvas::publishPointer`, an internal native that calls
  `emit_publish_pointer`. Like `MFB_CANVAS_DUMP`, a normal build contains no reader.
- **A proof that each platform's real handler publishes.** GTK and Windows gain
  `MFB_CANVAS_POINTER_X` / `_Y` in their headless paths, calling the same motion handler
  the platform signal calls, on the scripted-resize precedent. macOS has no such
  headless path; its handler is proved in a real window by `scripts/test-macapp.sh`.

**Gate class: new state with no visible effect yet.** Its observable is the stats line.
The four `app-mouse-surface` `.ncodesum` goldens are EXPECTED to change.

**Where the risk is:** three hand-written platform back ends, one of them with a
main-thread method table (`mouse_imps`) generated per view. A producer that publishes
after its range check would never report a leave.

### Rejected alternatives

- **Deriving the pointer on the graphics thread from the SGR pipe.** Rejected: the pipe
  has one reader, the worker, and only while it is in an `io::` read.
- **A lock around the words.** Rejected: the main thread must not block on the graphics
  thread, and a sequence word gives a consistent read without one.
- **Signalling a redraw on every move now.** Rejected: it would break the "static scene
  costs zero frames" rule for every program, before any feature needs it.

## Compatibility / Format Impact

- **Changed:** `GRAPHICS_STATE_SIZE` grows by 40 bytes; three producers gain a call and
  a leave handler; the debug stats line gains `pointer=`.
- **Added (test affordances):** `MFB_CANVAS_POINTER` (debug builds),
  `MFB_CANVAS_POINTER_X` / `_Y` (GTK and Windows headless).
- **Unchanged:** every public canvas function and type, the SGR event stream, every
  pixel.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — The words, the reader and the debug script

- [ ] Read the six mouse-mode tests and the tracking-area installer's caller; record
      where each sits, and correct §3 where a producer's shape differs from what it
      assumes.
- [ ] `src/codegen/runtime/canvas/mod.rs`: the five offsets, `GRAPHICS_STATE_SIZE`
      derived from the last, `emit_publish_pointer`, and the size assertion in
      `vulkan.rs`.
- [ ] `canvas::pointerSnapshot` and `canvas::publishPointer`, both `internal_only`, in a
      new `func_pointer.rs`.
- [ ] The debug-build `MFB_CANVAS_POINTER` reader in `helper_render.rs`, beside the
      stats hooks, and `pointer=` on the stats line.
- [ ] Extend `the_normal_source_has_no_reporting_hook` so a normal build is asserted to
      contain no `MFB_CANVAS_POINTER` reader.
- [ ] `tests/canvas/rt_canvas_pointer.rs`, new: a script of three entries and a program
      that presents three times; the three stats lines report the three pointer states.

Acceptance: the stats line reports each scripted pointer state, and a normal build has
no reader.
  Check: `cargo test --test rt_canvas_pointer --test rt_canvas_debug_hooks` → pass
  (est. 5 min).
Commit: —

### Phase 2 — macOS

- [ ] `mouse_view.rs`: call `emit_publish_pointer` in each of the move, drag, down and
      up methods; add entered-and-exited to the tracking area and a `mouseExited:`
      method; publish `INSIDE = 0` for an out-of-bounds point.
- [ ] `scripts/test-macapp.sh`: a case that opens a real canvas window with
      `MFB_CANVAS_STATS`, posts a pointer move into it and one out of it, and asserts
      the two `pointer=` values. If the script has no way to post a pointer event, add
      the smallest one under `tools/pointer-probe/` with a README.
- [ ] Regenerate the macOS `app-mouse-surface` goldens per `.ai/testing-gates.md`.

Acceptance: in a real window, a move inside reports inside with the right coordinates
and a move out reports outside.
  Check: `bash scripts/test-macapp.sh` → its new case passes (est. UNMEASURED — time
  the script once and record it; it is the only instrument that runs AppKit's real
  event path).
Commit: —

### Phase 3 — GTK and Windows

- [ ] `linux_gtk/mouse.rs`: publish in the four handlers; connect `leave`; the headless
      `MFB_CANVAS_POINTER_X` / `_Y` path calling the motion handler.
- [ ] `win_x86_64/app/mouse.rs`: publish in the eight arms; `TrackMouseEvent` and
      `WM_MOUSELEAVE`; the same headless path.
- [ ] A headless case in `tests/canvas/rt_canvas_pointer.rs` for the native env pair,
      skipped on macOS with the reason stated.
- [ ] Run it on box 2226 (GTK, native aarch64) and on `test@192.168.1.206` (the only
      Windows box).
- [ ] Regenerate the Linux and Windows `app-mouse-surface` goldens.
- [ ] `.ai/canvas-threading.md`: §1's table (main now also owns pointer publication) and
      a new subsection beside §5 stating the sequence-word rule; §11 gains the three
      environment variables.
- [ ] `src/docs/spec/app/06_canvas.md`: nothing user-visible changes in this letter;
      add the pointer words to the threading paragraph it already has, cited.

Acceptance: the native handler on each of GTK and Windows publishes the scripted
position.
  Check: `cargo test --test rt_canvas_pointer` on box 2226 and on the Windows box →
  pass (est. 20 min; the two boxes are the only places these back ends execute).
Commit: —

## Validation Plan

- Tests: `tests/canvas/rt_canvas_pointer.rs` (new); one `scripts/test-macapp.sh` case;
  the regenerated `app-mouse-surface` goldens on four targets.
- Coverage check: `scripts/artifact-gate.sh target/release/mfb canvas` (est. 1 min) —
  the producers are covered only by `app-mouse-surface`, so confirm it ran.
- Runtime proof: a canvas program blocked in `io::input`, run in a real window on each
  platform with `MFB_CANVAS_STATS`, its `pointer=` following the mouse.
- Doc sync: `.ai/canvas-threading.md`; `src/docs/spec/app/06_canvas.md`.
- Final gate: none in this letter; 172-M.

## Open Decisions

- **How the macOS handler is proved if `scripts/test-macapp.sh` cannot post a pointer
  event.** Recommended: a small event-posting helper under `tools/pointer-probe/`. The
  alternative is a native `MFB_CANVAS_POINTER_X` / `_Y` headless path on macOS too,
  which has no scripted-resize precedent there to build on.

## Corrections

<Filled in DURING execution.>

## Summary

Five words and one helper, called from three places that already know the coordinate.
The work is in the platforms: a leave event none of them reports today, and a proof of
each real handler, which is why this letter is separate from the feature that reads the
words.
