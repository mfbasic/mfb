# bug-993: macOS app mouse IMPs clobber callee-saved `x19`, so AppKit releases a garbage pointer and the app SIGSEGVs on a zero-`deltaY` scroll or an out-of-view mouse-up

Last updated: 2026-10-07
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Fixed (2026-10-07, bs18-agent-i)
Regression Test: `tests/runtime/rt_foreign_entry_callee_saved.rs` (`macos_app_imps_preserve_callee_saved`); `macos_aarch64::app::mouse_view::tests::every_imp_saves_and_restores_local0`

A macOS app-mode program that has turned mouse reporting on (`canvas::enableMouse(TRUE)`;
`term::enableMouse(TRUE)` installs the same IMP bodies) dies with SIGSEGV on the main thread
inside AppKit as soon as one of its mouse IMPs takes an **early exit**. The common trigger is a
trackpad or Magic Mouse: every gesture brackets its pixel deltas with began/ended events that
carry `deltaY == 0`, and a horizontal swipe has a zero `deltaY` all the way through. Releasing a
drag outside the view crashes the same way. The user hit it with a real trackpad in
`examples/canvas-demo`. With mouse reporting off, the IMPs exit before they touch the register
and the app survives.

**Correct behavior:** every Objective-C IMP the backend registers is an AAPCS64 callee and keeps
`x19`–`x28` (and `d8`–`d15`) unchanged. A scroll with a zero vertical delta, or a press or
release outside the surface, reports nothing and returns to AppKit with the caller's registers
intact. The program keeps running.

References:

- `.ai/arch-abi.md` § "macOS AArch64" and § "A hand-written pthread start routine must save the
  callee-saved registers it uses". This is the same class of bug, with AppKit as the caller.
- `.ai/codegen-invariants.md` bug-774: "a hand-written helper that writes `x21` without saving it
  would corrupt callers silently". `tests/runtime/rt_runtime_helper_callee_saved.rs` is the
  `-ncode` audit pattern this fix reuses.
- `planning/completed/plan-94-C-mouse-macos-app.md` introduced the IMPs (commit 1bf8ef5c3,
  2026-09-20). `planning/completed/plan-94-D-mouse-gtk-app.md` introduced the GTK siblings
  (0bd7f79be).
- `src/docs/spec/app/04_term-backend.md` specifies the mouse surface.
- Found while the user exercised `examples/canvas-demo` with a trackpad. Crash reports are in
  `~/Library/Logs/DiagnosticReports/canvas-demo-2026-10-06-12*.ips` and `cdscroll-*.ips`.

## Failing Reproduction

Project `/tmp/cdscroll`: `project.json` has `"mode": "app"`. `src/main.mfb`:

```
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT io
IMPORT os

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  app::setTitle("cdscroll")
  LET poll AS Boolean = collections::contains(os::args(), "mouse")
  IF poll THEN canvas::enableMouse(TRUE)
  MUT wheel AS Integer = 0
  MUT i AS Integer = 0
  DO WHILE i < 3000
    canvas::present([canvas::Rectangle[x := 0.0, y := 0.0, w := 400.0, h := 300.0, paint := canvas::fill(color::rgb(30, 60, 90))]])
    canvas::waitFrame()
    IF poll THEN
      MUT ev AS canvas::MouseEvent = canvas::pollMouse()
      DO WHILE ev.kind <> canvas::MouseKind.None
        IF ev.kind = canvas::MouseKind.ScrollUp OR ev.kind = canvas::MouseKind.ScrollDown THEN wheel = wheel + 1
        ev = canvas::pollMouse()
      LOOP
    END IF
    i = i + 1
  LOOP
  io::printError("wheel events " & toString(wheel))
  RETURN 0
END FUNC
```

The events are synthetic, posted with CoreGraphics. `/tmp/bug993/mouse993.swift` extends
`/tmp/cdtools/mouse.swift`. The case that isolates the trigger posts one continuous scroll event
per `dy:phase:momentum[:repeat]` token:

```swift
case "seq":
  post(.mouseMoved, p1)
  let units: CGScrollEventUnit = a[5] == "line" ? .line : .pixel
  for tok in a[6...] {
    let f = tok.split(separator: ":").map { String($0) }
    for _ in 0..<(f.count > 3 ? Int(f[3])! : 1) {
      let e = CGEvent(scrollWheelEvent2Source: nil, units: units, wheelCount: 1,
                      wheel1: Int32(f[0])!, wheel2: 0, wheel3: 0)!
      e.setIntegerValueField(.scrollWheelEventScrollPhase, value: Int64(f[1])!)
      e.setIntegerValueField(.scrollWheelEventMomentumPhase, value: Int64(f[2])!)
      e.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
      e.post(tap: .cghidEventTap); usleep(12000)
    }
  }
```

`/tmp/bug993/try.sh <mouse|nomouse> <tool args>` launches
`build/cdscroll.app/Contents/MacOS/cdscroll`, waits 2.5 s, activates it with
`/tmp/cdtools/activate <pid>`, runs the tool and prints `ALIVE` or `DIED rc=…`. It moves the real
pointer and briefly focuses the window. Pass multi-token arguments as separate words: zsh does
not word-split an unquoted `$var`, and the tool traps on a single merged token.

```
target/release/mfb build /tmp/cdscroll
/tmp/bug993/try.sh mouse   seq 0.5 0.5 pixel 0:1:0        # one zero-delta "began"
/tmp/bug993/try.sh nomouse seq 0.5 0.5 pixel 0:1:0 0:4:0
/tmp/bug993/try.sh mouse   dragout 0.5 0.5                # press inside, release above the window
```

- Observed: `Segmentation fault: 11`, `DIED rc=139`. The stack is
  `objc_release + 16 ← -[NSWindow(NSEventRouting) sendEvent:] + 288 ← -[NSApplication(NSEventRouting) sendEvent:] + 1504 ← _handleEvent: ← -[NSApplication run]`
  with `KERN_INVALID_ADDRESS at 0x20`. The drag-out crash faults instead at
  `_routeMouseUpEvent + 172 ← -[NSWindow _reallySendEvent:isDelayedEvent:] + 440` with
  `KERN_INVALID_ADDRESS at 0x105`. That matches the user's second crash.
- Expected: `ALIVE`. The zero-delta events report nothing and the out-of-view release reports
  nothing.

Measured matrix. Each row is one `try.sh` run on macOS 15.7.7 arm64 against the `demo` worktree
build. `mouse_view.rs`, `bootstrap.rs` and `linux_gtk/mouse.rs` are identical to `main` 468ff0f94
(checked with `git diff --stat 468ff0f94 -- <those files>`, which prints nothing).

| Events posted (`seq`/`variant` args) | mouse on | Result |
| --- | --- | --- |
| `trackpad` (4 rounds: 0-delta began, 20×±6 changed, 0-delta ended, momentum, 0-delta momentum end), run 6× | yes | DIED ✗ (6/6) |
| `trackpad` | no | ALIVE ✓ |
| `pixel 0:1:0` (a single zero-delta phased event) | yes | DIED ✗ |
| `pixel 0:4:0`, `pixel 6:2:0 0:2:0`, `pixel 3:0:2 0:0:2` (zero delta at phase end or in momentum) | yes | DIED ✗ |
| `pixel 6:0:0 0:0:0` (no phase, no momentum) | yes | DIED ✗ |
| `line 1:0:0 0:0:0` (line units, zero dy) | yes | DIED ✗ |
| `variant … line 0 0 0 0 1 5` (plain wheel, **horizontal only**, no phase, not continuous) | yes | DIED ✗ |
| `pixel 6:2:0:20`, `pixel 4:0:1 3:0:2:20` (all non-zero, phased or momentum) | yes | ALIVE ✓ |
| `variant … line 0 0 0 1 0 5`, `… -1 …`, `pixel 0 0 0 5 0 10` (non-zero dy, any units) | yes | ALIVE ✓ |
| `dragout` (release outside the view), run 2× | yes | DIED ✗ (2/2) |
| `dragout` | no | ALIVE ✓ |
| `click` inside the view | yes | ALIVE ✓ |

**Phase, momentum and continuous-ness do not matter. A zero `deltaY` does.** Any scroll event
whose `[event deltaY]` is `0.0` crashes, including a plain line-unit horizontal wheel tick, and
a non-zero one never crashes. The earlier finding that line-unit wheel clicks do not crash holds
only because those had a non-zero `deltaY`. Trackpads emit zero-delta began/ended events on every
gesture, which is why the user saw the crash with a trackpad and not with a mouse wheel.

Term surface, `/tmp/bug993/tm`, `Mode.Console` with `term::enableMouse(TRUE)`: zero-delta
scroll, non-zero scroll and drag-out were each run once and all came back **ALIVE**. The same
clobber is present (see Blast Radius) but did not crash in that window.

## Root Cause

`src/target/macos_aarch64/app/mouse_view.rs:emit_mouse_imp` builds all 22 IMPs (11 per surface).
Its prologue saves only `lr`, `self` and `event`. Its own doc comment says "`lr` and the
callee-saved registers the ObjC calls clobber are parked at the bottom", but no callee-saved
register is stored. Three helpers then write `abi::LOCAL[0]`, which is `x19`, callee-saved:

- `emit_to_surface_coords` (Pixels): `asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL)`,
  which loads the address of `_mfb_rt_canvas_graphics` into `x19`. (Cells:
  `move_register(LOCAL[0], c_return(0))`, the `TVSTATE` associated object.)
- `emit_button_code`: `move_register(LOCAL[0], c_return(0))`, the `modifierFlags` result.
- `emit_pipe_write`: `external_data(LOCAL[0], CLASS_NS_APPLICATION, …)`, the `NSApplication`
  class.

The epilogue at `{sym}_done` restores only `lr`, so the IMP returns to AppKit with `x19` replaced.
`-[NSWindow sendEvent:]` keeps an object it owns in `x19` across the dispatch and passes it to
`objc_release` afterwards. What it then releases depends on which exit the IMP took:

- **Early exit** (`deltaY == 0` → `branch_eq(done)` in `emit_button_code`'s `Wheel` arm, or a
  coordinate outside the surface → `emit_range_check` → `done`): `x19` still holds
  `&_mfb_rt_canvas_graphics`, a data address that is not an object. `objc_release` reads the
  zero first word as `isa` and dereferences `isa + 0x20`, which faults at `0x20`.
  `_routeMouseUpEvent` uses the same register in a different way and faults at `0x105`.
- **Full path** (non-zero delta, in-range click): the IMP ends in `emit_pipe_write`, so `x19`
  holds the `NSApplication` class object. `objc_release` on a class object is harmless, so these
  events survive by luck. AppKit's own object in `x19` is still never released (a leak per event;
  guess, not measured).

Evidence:

- In every crash report, `x0` at `objc_release` (the value from `x19`) is the image base plus
  `0x228018`. Commands: `cdscroll-2026-10-06-122018.ips` gives x0 `0x10110c018` with base
  `0x100ee4000`, and the drag-out report `cdscroll-2026-10-06-123217.ips` gives x19
  `0x104970018` with base `0x104748000`. `otool -tv` of the built binary shows the canvas
  `scrollWheel:` IMP doing `adrp x19, …0x100228000; add x19, x19, #0x18` (that is
  `_mfb_rt_canvas_graphics`, per the `-ncode` dump), then `fcmp d0, #0.0; b.eq` to an epilogue of
  only `ldr x30,[sp]; add sp,sp,#0x90; ret`.
- An `-ncode` audit flags every function that writes a callee-saved register before storing it:
  `mfb build -ncode /tmp/bug993/nc`, then `python3 /tmp/bug993/audit.py cdscroll.ncode`. On
  macos-aarch64 it reports exactly the 22 `macapp.mouse.{term,canvas}.*` IMPs (`['x19']`), plus
  the non-returning `_main`/`_mfb_macapp_program` entries and `_mfb_macapp_canvas_blit` (see
  Blast Radius).

Mouse-off programs are immune because the gate (`compare_immediate(SCRATCH[0], mode)` →
`branch_ne(done)`) runs before any write to `x19`. `keyDown:`/`keyUp:` IMPs are immune because
`keys.rs:emit_key_up` saves and restores `LOCAL[0]` at `[sp+8]`.

## Goal

- Every IMP produced by `emit_mouse_imp` preserves `x19`–`x28` on every exit path. The
  zero-`deltaY` and drag-out reproductions print `ALIVE`.
- A host-independent test fails if any OS-entered callback in a macOS or GTK app `-ncode` writes
  a callee-saved register it did not first save.

### Non-goals (must NOT change)

- SGR wire format, mouse-mode gating, coordinate conversion, the Y-flip, and the
  "zero `deltaY` reports nothing" rule. Do not "fix" the crash by reporting zero-delta scrolls.
- Do not move the `x19` writes onto the full path only, or reorder so the class pointer is always
  last. That keeps the clobber, and survival would still depend on what AppKit holds in `x19`.
- Do not make the IMPs return early for phased or momentum events. Phase is not the trigger.
- Do not touch the regression goldens beyond the `.ncodesum`/`.nir` deltas this fix produces.

## Blast Radius

Found by `python3 /tmp/bug993/audit.py` / `audit2.py` over the `-ncode` dumps of `/tmp/bug993/nc`
(a canvas+term mouse program) for macos-aarch64, linux-aarch64, linux-x86_64 and windows-x86_64.
Each script flags any function with an empty or `lr`-only `calleeSaved` frame that writes a
callee-saved register before storing it.

- `src/target/macos_aarch64/app/mouse_view.rs:emit_mouse_imp`, all 11 `MFBCanvasView` IMPs
  (`_mfb_macapp_canvas_{mouseDown,mouseUp,mouseDragged,rightMouse*,otherMouse*,mouseMoved,scrollWheel}`).
  **Fixed by this bug.** Runtime-confirmed for `scrollWheel:` and `mouseUp:`.
- Same function, all 11 `TermView` IMPs (`_mfb_macapp_term_*`). **Fixed by this bug** (same
  body). On an early exit `x19` holds the `TVSTATE` associated object, which AppKit then releases
  without owning it. The term repro survived one run of each trigger, so this is not observed to
  crash. A deferred over-release or use-after-free is a guess.
- `src/target/macos_aarch64/app/bootstrap.rs:emit_canvas_blit_helper`
  (`_mfb_macapp_canvas_blit`, worker thread, called from `_mfb_rt_canvas_canvas_blitSurface`). A
  **separate defect of the same class.** It stores `LOCAL[0,1,2,5]` but never `LOCAL[3]`/`LOCAL[4]`
  (`x22`/`x23`), and its epilogue reloads them from `[sp+32]`/`[sp+40]`, slots nothing wrote. Every
  `canvas::blitSurface` call therefore returns with `x22`/`x23` holding stale stack. Proof: in the
  `-ncode`, `_mfb_macapp_canvas_blit` has no `str_u64` with `src` x22/x23 and has
  `ldr_u64 x22,[sp,32]`/`x23,[sp,40]`. It is not observed to fail, because whether the MFB caller
  keeps a value live in x22/x23 across the call depends on regalloc. Include it in this fix: it is
  the same audit, the same test catches it, and the fix is a two-line prologue store.
- `src/target/linux_gtk/mouse.rs:emit_mouse_handler`
  (`_mfb_gtkapp_mouse_{pressed,released,motion,scroll}`). **Same hazard, latent.** It writes
  `LOCAL[0]`–`LOCAL[3]` (x19–x22) without saving them on linux-aarch64. On linux-x86_64,
  `finalize_x86_app_function` saves rbx/r12–r14 but the handler still writes `rbp` unsaved. GTK
  calls these from its signal marshaller. Not runtime-tested (needs a GTK box, see
  `.ai/remote_systems.md`). **In scope:** the same test catches it, and leaving a found clobber is
  disallowed.
- `src/target/win_x86_64/app/mod.rs` `winapp.wndproc` mouse arms: **unaffected.** The audit
  reports nothing for `winapp.wndproc`. The Windows hits (`runtime.canvas.enableMouse` → `rbx`,
  `#canvas_*` → `rdi`) are MFB-internal functions under the MFB register convention, not OS
  callbacks, so they are out of scope here.
- `_main` (macOS), `_mfb_gtkapp_main`, `_mfb_macapp_program`, `winapp.bootstrap`: **unaffected.**
  They end in `_exit` / `branch_self` and never return to their caller.
- `keys.rs:emit_key_up` / `keyDown:` IMPs, `term_view.rs`, `metal.rs`, `display.rs`,
  `window.rs` callbacks: **unaffected.** The macos-aarch64 audit flags none of them. This covers
  the functions emitted for a canvas+term program only.

## Fix Design

Give `emit_mouse_imp` a real callee-saved save area. Add `OFF_SAVED_X19` (or reuse an
8-byte slot; the frame is already 16-aligned, so check that `FRAME` grows by 16 if needed). Store
`abi::LOCAL[0]` in the prologue next to `lr`, and reload it in the `{sym}_done` epilogue, which is
the single exit every path, early or full, already funnels through. That is the
`keys.rs:emit_key_up` shape. Do the same in `linux_gtk/mouse.rs:emit_mouse_handler` for
`LOCAL[0]`–`LOCAL[3]` (the x86 transpile then saves the realized `rbp` too). In
`emit_canvas_blit_helper`, add the two missing stores of `LOCAL[3]`/`LOCAL[4]` at `[sp+32]`/`[sp+40]`.

Rejected alternatives:

- Replacing every `LOCAL[0]` use in the IMP with caller-saved scratch. This is fragile:
  `load_selector` and each `objc_msgSend` clobber x0–x17 (`.ai/arch-abi.md` "load_selector
  clobbers caller-saved"), so values would need stack round-trips. The save/restore is two
  instructions and mirrors the existing keys pattern.
- Declaring `callee_saved: vec![LOCAL[0]]` in the `CodeFrame` and relying on the finalizer. These
  hand-written bodies use `stack_size: 0` and their own frame. Check whether
  `finalize_frame` honours `callee_saved` for them before choosing this; if it does, it is
  acceptable but must produce the same runtime result.

Expected golden shift: `.ncodesum`/`.nir` for every macOS and GTK app fixture that emits mouse
IMPs or the canvas blit, at least `tests/syntax/app/app-mouse-surface/golden/*.{macos-aarch64,linux-aarch64,linux-x86_64}.app.*`
and any canvas app fixture with a macOS app golden. The delta must be confined to the mouse IMP
symbols and `_mfb_macapp_canvas_blit`. Windows goldens must not move.

## Phases

### Phase 1: failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_app_callback_callee_saved.rs` (landed as `rt_foreign_entry_callee_saved.rs`), modelled on
      `rt_runtime_helper_callee_saved.rs`. Build `-ncode` for a canvas+term mouse program
      (the `app-mouse-surface` source works) for macos-aarch64, linux-aarch64 and linux-x86_64 in
      app mode. For every function the OS enters (`_mfb_macapp_{term,canvas}_*` mouse IMPs,
      `_mfb_gtkapp_mouse_*`) and for `_mfb_macapp_canvas_blit`, assert that each callee-saved
      register it writes is stored to the stack before its first write and reloaded before every
      `ret`. Confirm it fails today, naming those functions and nothing else.
- [ ] Add a static IMP-shape assertion in `mouse_view.rs`'s test module: the `{sym}_done`
      epilogue reloads `LOCAL[0]`.
- [ ] Record each site's verdict in Blast Radius. Extend the audit to the other app-mode IMP
      emitters (`term_view.rs`, `metal.rs`, `window.rs`, `display.rs`) under a term-only program,
      so the "unaffected" verdicts cover every IMP.

Acceptance: the new test fails for exactly the sites listed in Blast Radius as fixed by this bug.
Commit: —

### Phase 2: the fix

- [ ] `src/target/macos_aarch64/app/mouse_view.rs:emit_mouse_imp`: save and restore `LOCAL[0]`.
      Correct the frame-layout doc comment so it states what is actually saved.
- [ ] `src/target/linux_gtk/mouse.rs:emit_mouse_handler`: save and restore `LOCAL[0]`–`LOCAL[3]`.
- [ ] `src/target/macos_aarch64/app/bootstrap.rs:emit_canvas_blit_helper`: store `LOCAL[3]`/`LOCAL[4]`.
- [ ] Add a short "app IMPs/callbacks are AAPCS callees" note to `.ai/arch-abi.md` § macOS
      AArch64, next to the pthread start-routine rule.

Acceptance: the Phase 1 test passes. The repro matrix rows that died now print `ALIVE`, and the
ALIVE rows still do. Wheel reports are unchanged: the cdscroll `wheel events N` count for the
`trackpad` run is non-zero and the same order of magnitude as before.
Commit: —

### Phase 3: regenerate expected outputs + full validation

- [ ] Regenerate only the moved `.ncodesum`/`.nir` goldens. Diff each and confirm the delta is
      confined to the mouse IMPs, the GTK mouse handlers and `_mfb_macapp_canvas_blit`.
- [ ] Run the full suite and `scripts/artifact-gate.sh` (full, not one module).
- [ ] Re-run the repro matrix, `examples/canvas-demo` with a real trackpad, and
      `scripts/test-macapp.sh`. Run a GTK mouse smoke on an aarch64 and an x86_64 box
      (`scripts/test-appimage.sh`).

Acceptance: full suite green, golden deltas exactly as intended, and every row that died in the
reproduction matrix now prints `ALIVE`.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_app_callback_callee_saved.rs` (landed as `rt_foreign_entry_callee_saved.rs`) (fails before the fix, passes
  after), plus the static IMP-shape assertion in `mouse_view.rs`.
- Runtime proof: `/tmp/bug993/try.sh mouse seq 0.5 0.5 pixel 0:1:0`,
  `… line 1:0:0 0:0:0`, `… dragout 0.5 0.5` and `/tmp/cdtools/mouse cdscroll trackpad 0.5 0.5`
  all leave the process alive. A real trackpad in `examples/canvas-demo` no longer crashes.
- Doc sync: `.ai/arch-abi.md` note. No `mfb spec`/`mfb man` change, because the observable
  contract is unchanged.
- Full suite: `cargo test`, `scripts/artifact-gate.sh`.

## Open Decisions

- Fold `_mfb_macapp_canvas_blit` and the GTK handlers into this bug, or file them separately?
  Recommended: fold them in. It is the same class, the same test and a two-to-four-line fix each,
  and filing them separately would leave a test that is known to be red.

## Summary

The fix itself is a few lines. The risk is in the audit: proving every OS-entered callback, on
every app backend, preserves callee-saved registers. A clobber that happens to leave a harmless
value (here, the `NSApplication` class on the full path) hides the bug until an early-exit path
leaves a non-object in the register. Wire format, gating and coordinate semantics stay untouched.

## Resolution

`emit_mouse_imp` (`src/target/macos_aarch64/app/mouse_view.rs`) stores `LOCAL[0]` (`x19`)
at `OFF_SAVED_LOCAL0` (72) in the prologue and reloads it at `{sym}_done`, the single exit;
the report buffer moved from 72 to 80 and the frame size did not change (rounding). The
frame doc comment now says what is saved. `emit_canvas_blit_helper`
(`app/bootstrap.rs`) now stores `LOCAL[3]`/`LOCAL[4]` at `[sp+32]`/`[sp+40]`, the slots its
epilogue already reloaded. The function-level `-ncode` diff against main is exactly the
save, the restore and the report-buffer offset (+8) in each of the 22 IMPs, and the two
stores in the blit. The audit (Phase 1's test) landed as
`tests/runtime/rt_foreign_entry_callee_saved.rs` under bug-982, which covers every
foreign-entered function on every app target; see bug-982's resolution for the full
verdict table. `.ai/arch-abi.md` has the note.

### Proof

Local macOS 15.7.7 arm64, `/tmp/bug993/try.sh` against `/tmp/cdscroll` built by each
compiler:

| events | main f6b4099c9 | fix |
| --- | --- | --- |
| `seq 0.5 0.5 pixel 0:1:0 0:4:0` | DIED rc=139 (2/2) | ALIVE |
| `seq 0.5 0.5 line 1:0:0 0:0:0` | DIED rc=139 (2/2) | ALIVE |
| `seq 0.5 0.5 pixel 0:1:0` | ALIVE once, then see above | ALIVE |
| `seq 0.5 0.5 pixel 6:0:0 0:0:0` | (doc: DIED) | ALIVE |
| `dragout 0.5 0.5` | DIED rc=139 | ALIVE (2/2) |
| `click 0.5 0.5` | (doc: ALIVE) | ALIVE |
| `trackpad 0.5 0.5` | (doc: DIED 6/6) | ALIVE (3/3) |
| `nomouse` | ALIVE | ALIVE |

Wheel count: `wheel events 0` with BOTH compilers on this host. That is not this fix:
`cdscroll` never reads stdin, and the mouse decoder runs only on the stdin read path (see
bug-982's Note); an lldb trace also showed no IMP reaching `write` under the synthetic
events here. The report path's code is unchanged except the buffer offset, per the
`-ncode` diff above. The GTK runtime proof on 2226 (bug-982) shows the shared decoder
delivering reports once the program reads stdin.

Not re-run: a real trackpad in `examples/canvas-demo` (needs a person at the machine).

## Golden impact

`syntax/app/app-mouse-surface/golden/app_mouse_surface.macos-aarch64.app.ncodesum` (the 22
IMPs, the blit and the graphics entry). No `.nir`/`.nplan` change (artifact gate). Windows
goldens move only through bug-982's graphics-entry/trampoline saves.
