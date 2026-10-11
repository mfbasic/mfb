# plan-172-J: The main thread publishes the pointer to the graphics state

Last updated: 2026-10-11
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
| plan-172-I complete and archived | `ls planning/plan-172-I-*` → no matches | MET — `no matches found`, 2026-10-11 |
| The Windows box is reachable | `ssh test@192.168.1.206 echo ok` → `ok` | MET — `ssh test@192.168.1.206 ver` → `Microsoft Windows [Version 10.0.26200.9457]` |
| Box 2226 has its GNOME Wayland session | probe per `.ai/remote_systems.md` | MET — `/run/user/1001/wayland-0` exists and `loginctl` lists session 7008 on seat0; its screen shield was up (Corrections) |

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

- [x] Read the six mouse-mode tests and the tracking-area installer's caller; record
      where each sits, and correct §3 where a producer's shape differs from what it
      assumes.
      → Three tests, not six (the count was three `use` lines and three uses). In every
      producer the gate is the FIRST act, before any coordinate exists — and, the
      finding that reshaped the letter, every producer is compiled in only for a
      program that uses a mouse member (`uses_mouse`). The tracking area likewise. See
      Corrections.
- [x] `src/codegen/runtime/canvas/mod.rs`: the five offsets, `GRAPHICS_STATE_SIZE`
      derived from the last, `emit_publish_pointer`, and the size assertion in
      `vulkan.rs`.
      → `GRAPHICS_OFFSET_POINTER_SEQ` / `_X` / `_Y` / `_BUTTONS` / `_INSIDE`,
      `GRAPHICS_POINTER_WORDS`, `PointerRegs`; `vulkan.rs:
      the_pointer_words_end_the_graphics_state`.
- [x] `canvas::pointerSnapshot` and `canvas::publishPointer`, both `internal_only`, in a
      new `func_pointer.rs`.
- [x] The debug-build `MFB_CANVAS_POINTER` reader in `helper_render.rs`, beside the
      stats hooks, and `pointer=` on the stats line.
      → `POINTER_SCRIPT` / `POINTER_SCRIPT_DEBUG`, a `debug_split` pair;
      `__canvas_pointerText` on `__canvas_writeStats`'s line.
- [x] Extend `the_normal_source_has_no_reporting_hook` so a normal build is asserted to
      contain no `MFB_CANVAS_POINTER` reader.
      → And no `canvas::publishPointer` call: the script is its only MFBASIC caller.
- [x] `tests/canvas/rt_canvas_pointer.rs`, new: a script of three entries and a program
      that presents three times; the three stats lines report the three pointer states.
      → `the_graphics_thread_reads_each_scripted_pointer_state`, and
      `an_unscripted_pointer_is_outside_and_due_entries_apply_in_order`.

Acceptance: the stats line reports each scripted pointer state, and a normal build has
no reader.
  Check: `cargo test --test rt_canvas_pointer --test rt_canvas_debug_hooks` → pass
  (est. 5 min).
  → `rt_canvas_pointer`: `2 passed; 0 failed`; `rt_canvas_debug_hooks`: `2 passed`;
  `cargo test --bin mfb -- the_normal_source_has_no_reporting_hook` green in the
  round's `--bin mfb canvas` (`118 passed; 0 failed`).
Commit: —

### Phase 2 — macOS

- [x] `mouse_view.rs`: call `emit_publish_pointer` in each of the move, drag, down and
      up methods; add entered-and-exited to the tracking area and a `mouseExited:`
      method; publish `INSIDE = 0` for an out-of-bounds point.
      → All eleven canvas IMPs publish (the wheel too: it has the position), plus
      `mouseEntered:` and `mouseExited:`, which only publish. The publish is BEFORE the
      mode gate, which moved; and the IMPs, their selectors, the tracking area and
      `NSTrackingArea` are now emitted for every canvas program, mouse member or not.
- [x] `scripts/test-macapp.sh`: a case that opens a real canvas window with
      `MFB_CANVAS_STATS`, posts a pointer move into it and one out of it, and asserts
      the two `pointer=` values. If the script has no way to post a pointer event, add
      the smallest one under `tools/pointer-probe/` with a README.
      → Case 6c, with `tools/pointer-probe/pointer-probe.c`. It asserts six states, not
      two: inside, left pressed, dragged off the surface with the button held (outside),
      dragged back, released, and outside after leaving.
- [x] Regenerate the macOS `app-mouse-surface` goldens per `.ai/testing-gates.md`.

Acceptance: in a real window, a move inside reports inside with the right coordinates
and a move out reports outside.
  Check: `bash scripts/test-macapp.sh` → its new case passes (est. UNMEASURED — time
  the script once and record it; it is the only instrument that runs AppKit's real
  event path).
  → Case 6c run ALONE, from the script's own text (its preamble, the case, its
  footer) with `MFB_MACAPP_GUI=1`: `ok: the canvas view published the pointer: inside,
  pressed, dragged off, released, outside`. The whole script was NOT run with its GUI
  legs, and so not timed: two of them type keystrokes into whatever window is frontmost,
  and the machine was in use (Corrections). By hand, the same window: `pointer=300,200,0,1`,
  `310,210,0,1`, `310,210,1,1`, `320,220,1,1`, `1050,220,1,0`, `330,230,1,1`,
  `330,230,0,1`, `0,0,0,0`.
Commit: —

### Phase 3 — GTK and Windows

- [x] `linux_gtk/mouse.rs`: publish in the four handlers; connect `leave`; the headless
      `MFB_CANVAS_POINTER_X` / `_Y` path calling the motion handler.
      → NOT in `mouse.rs`'s handlers. A new `linux_gtk/pointer.rs`: a motion controller
      (`motion`, `enter`, `leave`) and a click gesture on the canvas DRAWING AREA, which
      only publish (Corrections). The headless path is
      `bootstrap.rs:emit_headless_scripted_pointer`, calling the motion callback.
- [x] `win_x86_64/app/mouse.rs`: publish in the eight arms; `TrackMouseEvent` and
      `WM_MOUSELEAVE`; the same headless path.
      → NOT in the eight arms. A new `win_x86_64/app/pointer.rs`: two functions and a
      `WndProc` block that runs before the SGR arms and passes every message on
      (Corrections). The headless path calls `_mfb_winapp_pointer_move`.
- [x] A headless case in `tests/canvas/rt_canvas_pointer.rs` for the native env pair,
      skipped on macOS with the reason stated.
      → `the_platform_callback_publishes_a_scripted_position`,
      `#[cfg(not(target_os = "macos"))]`. It compiles there and is not in the macOS
      test list; it has not been RUN by cargo, because no box builds this tree
      (Corrections). The same three runs are `scripts/test-canvas-pointer.sh`.
- [x] Run it on box 2226 (GTK, native aarch64) and on `test@192.168.1.206` (the only
      Windows box).
      → `scripts/test-canvas-pointer.sh target/release/mfb --target <t>`, three rows
      each, all `ok`, on linux-aarch64 (box 2226), linux-x86_64 (box 2227, musl) and
      windows-x86_64 (box 206). And in REAL windows on both, by hand — see the
      Acceptance.
- [x] Regenerate the Linux and Windows `app-mouse-surface` goldens.
      → `scripts/artifact-gate.sh target/release/mfb all` → `2322 golden(s) checked, 0
      diff(s)`. Seven `app-mouse-surface` goldens and
      `loop-body-resource-reference-valid`'s `.ir` moved; nothing else.
- [x] `.ai/canvas-threading.md`: §1's table (main now also owns pointer publication) and
      a new subsection beside §5 stating the sequence-word rule; §11 gains the three
      environment variables.
      → §1, §5.1, §11.
- [x] `src/docs/spec/app/06_canvas.md`: nothing user-visible changes in this letter;
      add the pointer words to the threading paragraph it already has, cited.
      → A section of its own, "The pointer, as the graphics thread sees it": the words,
      the sequence rule, each platform's handlers, the three variables.
      `src/docs/spec/linker/03_import-selection.md` for the imports that follow the
      handlers.

Acceptance: the native handler on each of GTK and Windows publishes the scripted
position.
  Check: `cargo test --test rt_canvas_pointer` on box 2226 and on the Windows box →
  pass (est. 20 min; the two boxes are the only places these back ends execute).
  → `scripts/test-canvas-pointer.sh`, as above: on each of the three targets `the
  platform callback published (123,45) as inside` and `(5000,45) as outside`.
  Beyond the acceptance, in real windows (`tools/pointer-probe`):
  - GNOME Wayland, box 2226: `pointer=651,429,0,1` → `…,1,1` (left) → `661,439,1,1`
    (drag) → `…,0,1` → `…,4,1` (right) → `…,0,1` → `…,2,1` (middle) → `…,0,1` →
    `…,1,1` → `1090,439,1,0` (dragged off, held) → `-189,439,1,0` → `-189,439,0,0`.
  - Windows, box 206: `300,200,0,1` → `310,210,0,1` → `310,210,1,1` → `320,220,1,1` →
    `320,220,0,1` → `320,220,4,1` → `320,220,0,1` → `320,220,2,1` → `320,220,0,1` →
    `0,0,0,0` (`WM_MOUSELEAVE`).
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

  **Taken: the recommended one** (2026-10-11). `tools/pointer-probe/pointer-probe.c`.
  The alternative was not open in practice: a canvas IMP's first act is a message to
  the `NSEvent` it was handed, so there is no "same handler" a headless path could call
  without one.

## Corrections

Found during execution, 2026-10-11:

- **§2 was wrong about what exists: no producer runs in a program that does not use a
  mouse member.** "Each producer runs on the main thread" — only when `uses_mouse`. On
  macOS the 22 IMPs, their selector strings, the tracking area and the
  `NSTrackingArea` import were all emitted `if spec.uses_mouse`; on GTK the four
  handlers and the controllers; on Windows the eight arms. A canvas program with no
  mouse member had no pointer handler of any kind, so "written whether or not the
  program called `canvas::enableMouse`" was not a call to add at three sites: it was
  handlers for every canvas program. `app_mode_imports` and `app_mode_data_objects`
  take `uses_canvas` beside `uses_mouse` for it, across every platform's
  implementation, so each emits exactly the strings and imports its handlers name.
  `_mfb_rt_mouse_mode` is still a mouse program's alone, so a publish-only handler must
  not name it — `mouse_view.rs`, `pointer.rs` (both) have a test each.
- **§3's "each producer calls it … before its mouse-mode test" fits no producer.** The
  gate is the first instruction in all three; on GTK and Windows it also CHOOSES the
  surface (cells or pixels) the handler converts into. So:
  - macOS: the gate moved. A canvas IMP now locates, converts, publishes, and then
    gates; in a program with no mouse member it is the publish and a return.
  - GTK and Windows: the reporting code is not touched at all. Publication is separate
    code — on GTK, controllers on the canvas drawing area, where GTK's coordinates are
    already the surface's and `leave` is the surface's; on Windows, a block ahead of the
    SGR arms. That kept "the SGR pipe … untouched" true by construction on two of the
    three.
- **macOS takes the buttons from the event, not only from AppKit's current state.**
  `+[NSEvent pressedMouseButtons]` is the hardware NOW; a quick click is released
  before its `mouseDown:` is dispatched. A press or drag sets its own button's bit and a
  release clears it. AppKit orders the bits left, right, other; the published mask is
  left, middle, right.
- **On GTK the held mask cannot be read from GDK's event state. Measured, in a real
  window.** First cut: `(state >> 8) & 7`, corrected by the event's button. On GNOME
  Wayland a right press published 4 and its release published 2; a middle press 2 and
  its release 4. GDK's Wayland backend numbers a press's `button` left 1, middle 2,
  right 3 but sets the state bit by evdev order — left, right, middle — so bits 9 and
  10 are the reverse of `GDK_BUTTON2_MASK` / `GDK_BUTTON3_MASK` (and of X11). The mask
  is now kept in `_mfb_gtkapp_pointer_held` from the button number of each press and
  release, and the state answers only "is any button held".
- **A point off the surface is published, on all three, and that is what reports a
  drag leaving.** None of the three sends a leave event while a button is held.
- **Headless, "inside" needs a published size.** The words a bounds test reads
  (`GRAPHICS_OFFSET_WIDTH` / `_HEIGHT`) are 0 until a window publishes them, and a
  headless run has no window: the first box run published `123,45,0,0`. The scripted
  pointer therefore runs AFTER the scripted resize, and a test that wants "inside"
  scripts both.
- **The check is a script, not `cargo test` on a box.** No box builds this tree (2226
  only through a user-space toolchain, the Windows box not at all). The canvas rows on
  the boxes have always been cross-built here and shipped
  (`scripts/test-canvas-gpu-rows.sh`); `scripts/test-canvas-pointer.sh` is the same
  shape, and adds a row the plan did not ask for — the graphics-thread script on each
  target, which is where the sequence word's release stores (AArch64) and plain stores
  (x86-64) are actually executed.
- **Real windows, on all three, and what they found.** The plan asked for a real window
  on macOS only and accepted a headless call for the other two. `tools/pointer-probe`
  has a driver for each window system, and the first real GTK window found three things
  the headless call could not have: the state-bit order above, and two bugs that were
  already there —
  - **bug-1043 (fixed):** a canvas program's GTK window had no mouse controllers at
    all. They were attached where a `Console`-default program's window is built and not
    where an `app::setMode` program's is. `canvas::pollMouse` reported nothing in a real
    GTK window, ever. A/B at `6a473b1c7`: zero events before, events after.
  - **bug-1044 (fixed):** GTK reported only the left button. The click gesture
    listened to the primary button alone.
  - **bug-1045 (OPEN):** on Windows, a real window on the GPU renderer writes no
    `MFB_CANVAS_STATS` line. The same at `6a473b1c7`. Not root-caused; the Windows
    real-window proof above therefore ran with `MFB_CANVAS_GPU=0`.
- **`scripts/test-macapp.sh` was not run whole with its GUI legs.** Case 6c was run
  alone, from the script's own text. Cases 6 and 6b inject keystrokes with System
  Events into whatever is frontmost, the machine had other sessions' work on it, and a
  stray `WindowKeys` in someone's editor is not a cost this letter gets to impose. So
  the script's total time with the new case is still UNMEASURED.
- **The GTK and Windows real-window runs are by hand, not a script leg.** Their drivers
  and their traps are in `tools/pointer-probe/README.md`. One of the GNOME runs saw no
  pointer at all: a window that is not fullscreen does not always open in the same
  place, and Wayland tells a client neither where it is nor where the pointer is.
- **Box 2226's session had its screen shield up**, which makes Mutter refuse a
  remote-desktop session ("Session creation inhibited"). Lifted with
  `org.gnome.ScreenSaver.SetActive false`; nothing on the box was reconfigured.
- **The first cut of the script test had 100 ms margins and failed.** A `--debug`
  software frame is slow enough that the third entry landed in the second frame. The
  entries are now seconds apart, and the clock starts at the first frame.
- **Three existing tests changed, each for a reason that is not "to pass":**
  - `macos_aarch64/app/mod.rs:canvas_view_overrides_the_two_methods_that_deliver_keys`
    asserted exactly five `class_addMethod` calls for a canvas view in a program with no
    mouse member — "one per override, and no more". This letter installs thirteen
    pointer methods on that view by design. The count is five plus
    `mouse_imps(Pixels).len()`, that length is pinned at 13, and `mouseMoved:` and
    `mouseExited:` are in the list of selectors it names with a reason each. Found by
    the full round, not by the pointer tests.
  - `tests/codegen/codegen_mouse_arena_region.rs:
    a_canvas_program_that_never_polls_the_mouse_reserves_no_mouse_state` — 172-I's own
    test, one letter old. It asserted that no data object's name contains `mouse`, as a
    stand-in for "no mode word". Every canvas program now has `_mfb_macapp_sel_mouseDown`
    and its siblings, by this letter's design; the assertion is now the claim itself,
    no `_mfb_rt_mouse_mode`.
  - `win_x86_64/app/mod.rs:canvas_hwnd_is_published_by_one_arm_and_cleared_by_the_others`
    counts the window procedure's reads of the canvas handle, and names each reader.
    The pointer block gates on it twice; the count is 18, not 14, and the message names
    the two.
- **`GRAPHICS_STATE_SIZE` grew by 40 bytes**, as planned; and two more data objects in
  a canvas program the plan did not list: GTK's held-button word and Windows' "a leave
  is armed" word.
- **On Windows `MFB_CANVAS_POINTER_X` / `_Y` of 0 is "unset"**: `emit_parse_wide_env`
  answers 0 for a variable that is absent, so the hook cannot tell them apart. The
  scripted pointer cannot be put on the surface's first row or column there.

## Summary

Five words and one helper, called from three places that already know the coordinate.
The work is in the platforms: a leave event none of them reports today, and a proof of
each real handler, which is why this letter is separate from the feature that reads the
words.

As built: the helper is as planned, and the "three places" did not exist for the
programs that need them — so each platform gained pointer handlers for every canvas
program, and a real window on each proved them. Those windows also found two bugs in
the reporting path that had been there since it was written.
