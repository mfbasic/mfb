# bug-1015: on GTK, `term::pollMouse` sees no mouse events unless the program also reads stdin

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (platform parity)

Status: Fixed
Regression Test: (proposed) a GTK app-mode row in `scripts/test-appimage.sh`: `term::enableMouse(TRUE)` + a `term::pollMouse` loop with NO stdin read, driven by xdotool click/move/scroll under Xvfb, asserting at least one event per action

## Cause (coordinator, bug-staging-24)

- Rule: `pollMouse` returns the events the window received, whether or not the program reads stdin (owner decision 2026-10-10), on every platform.
- Where: the mouse handlers on GTK and Windows (`src/target/win_x86_64/app/mouse.rs`) write SGR reports into the worker's input stream, and the only decoder runs on the `io` read path (`src/codegen/builtins/io/gen_read_family.rs`, around `mouse_decode::emit_drain_pending`); `term::pollMouse`/`canvas::pollMouse` only dequeue the ring (`src/codegen/io/mouse/ring.rs:emit_dequeue`). This doc's own Windows measurement shows the cause is shared.
- Fix: `pollMouse` drains and decodes pending input itself before it dequeues, through the same drain the read family uses, keeping every non-mouse byte for the next `io` read; fixes every platform at once. No per-platform delivery path is added.

**Owner decision (2026-10-10): GTK mouse events come from the window, like macOS and Windows.**
`term::pollMouse` must return the events the window received, whether or not the program ever
reads stdin.

On Linux GTK `--app` builds the mouse handlers write an SGR mouse report into the terminal input
stream, and the decoder that turns reports into `MouseEvent`s runs only on the stdin read path
(`io::readKey` and friends). A program that polls the mouse without reading stdin never decodes
anything, so `term::pollMouse` returns `MouseKind.None` forever.

**Correct behavior:** the GTK window's press/release/motion/scroll events reach the mouse event
queue directly (or `pollMouse` drains and decodes pending input itself), so the same program
behaves the same on GTK, macOS and Windows. Check how the macOS and Windows backends feed the
queue and make GTK match.

## Failing Reproduction

Measured by the coordinator on 2026-10-10 on box 2226 (Debian 12 aarch64 glibc, GTK4, Xvfb +
xdotool), with bs18-agent-i's compiler (bug-982 fixed, so the app no longer crashes). The program
is bug-982's repro: `term::on`, `term::enableMouse(TRUE)`, then 40 × (`term::pollMouse`, log an
`event` line when the kind is not `None`, `os::sleep(100)`), then `done`. It never reads stdin.

| action | `event` lines | `done` |
|---|---|---|
| `xdotool click 1` | 0 | 1 |
| `xdotool mousemove …` | 0 | 1 |
| `xdotool click 4` (scroll) | 0 | 1 |

bs18-agent-i measured the same program with one `io::readKey(0)` added per loop: 2, 2 and 3 events.
(The coordinator has not re-run that variant.)

Expected: at least one `event` line per action.

## Windows does the same (measured 2026-10-11, plan-172-M)

The owner decision above takes Windows as the model GTK should match. In a real canvas
window on box 206 (Windows 11) it behaves as GTK does: `canvas::pollMouse` reports
nothing until the program also reads the keyboard.

`bugs/repro/bug-1015/overid.mfb` presents a small scene, calls `canvas::enableMouse(TRUE)`
and polls `canvas::pollMouse` for 14 s, appending every event to a file; with
`READ_KEYS=1` it also calls `io::readKey(0)` once a loop. `overid.ps1` starts it, puts
the REAL cursor on four client points with `SetCursorPos` and presses and releases the
left button at each. Cross-built `--app --target windows-x86_64` at `8bb3e9a60` and run
inside the logged-in desktop session (a scheduled task with `/it`):

| `READ_KEYS` | what the program recorded |
|---|---|
| unset | `started 384x261`, then `moves 0` — no move, no press, no release in 14 s |
| `1` | twelve events, a `move`, a `down` and an `up` at each of the four points |

The window received every message both times: the pointer words the graphics thread
reads (plan-172-J) are written by the same `WndProc` and were moving. The cause is the
one above — `src/target/win_x86_64/app/mouse.rs` writes SGR reports to the worker's
input pipe, and the only decoder is in the `io` read family
(`src/codegen/builtins/io/gen_read_family.rs`, the pump around
`mouse_decode::emit_drain_pending`); `canvas::pollMouse` only takes from the ring
(`src/codegen/io/mouse/ring.rs:emit_dequeue`). So the fix that makes `pollMouse` drain
and decode pending input itself fixes every platform at once, and "make GTK match
Windows" is not a fix.

`mfb man canvas pollMouse` and `enableMouse` say nothing about needing a key read.

## Related

- bug-982 (the crash these handlers used to cause; fixed). bug-985 (a `setMode`-built window
  attaches no mouse controllers at all): fix this one first or together, since both end in
  "pollMouse sees nothing".
- macOS: bs18-agent-i could not measure synthetic mouse delivery on the host at all (no IMP
  reached `write`, `screencapture` returned black), so macOS delivery is unverified, not known good.

## Fix

- **What changed.** New `src/codegen/builtins/io/gen_read_family.rs:emit_decode_pending_input`
  runs the read family's own source read (`emit_stdin_byte_read`, same mouse pump, same
  `ReadWait`) with a deadline of "now" until nothing is ready: reports are decoded into
  the ring by the pump; every other byte it delivers is appended to the input buffer
  (room checked BEFORE each read, so a full buffer stops the drain without losing a
  byte); EOF / read error / unsubscribed thread end the drain silently. It is called at
  the top of `src/codegen/io/mouse/ring.rs:emit_dequeue`, which both `pollMouse` bodies
  (`term::pollMouse`, `canvas::pollMouseRaw`) dequeue through, so no per-platform path
  was added. Skipped while the mouse-mode word is 0, and in a console build with no
  input buffer (no `io` reader ⇒ no broadcast log/subscription to read through; see
  "Not fixed" below). A program with no input buffer (no `io` reader at all) has no
  later read to keep keystrokes for; in an app build those bytes are discarded.
  Frame slots: `term::pollMouse` gets its own `POLL_MOUSE_LOCALS_SIZE` (LOCALS_SIZE +
  40); `canvas::pollMouseRaw` allocates a 40-byte stack object. Imports: the pollMouse
  rows now declare the read primitives (linux `read`/`__errno_location`, macOS
  `_read`/`___error`, Windows `GetStdHandle`/`ReadFile`/`GetFileType`/`PeekNamedPipe`/
  `WaitForSingleObject`/`Sleep`/`GetLastError` — without them a reader-less Windows app
  failed to build: "external relocation target 'GetFileType' is not imported").
- **Tests (RED with `MFB_TEST_EXE=/tmp/main-dbg-target/release/mfb`, GREEN with the fix):**
  - `tests/runtime/rt_native_term_runtime.rs`:
    `native_term_poll_mouse_decodes_without_a_prior_read_and_keeps_the_keys` (console,
    stdin `ab⟨report⟩cd⟨report⟩ef`, poll before any read: main `EVENTS:` → fixed
    `EVENTS:[1,3][4,6]`, then `CHARS:abcdef`) and
    `app_term_poll_mouse_decodes_without_a_prior_read_and_keeps_the_keys` (same program,
    headless `--app`, stdin = the window input stream).
  - `tests/canvas/rt_canvas_hit.rs:poll_mouse_decodes_the_window_reports_without_a_read`
    (headless canvas, reports on stdin, no reader: main `over` → fixed `over [box] []`).
  - `scripts/test-appimage.sh --gui` row "pollMouse sees window events with no stdin
    read": main `events=0`, fixed `events=1` (`MFB_APPIMAGE_DISPLAY=:77 … --box 2226
    --libc glibc --gui`).
- **Measured on real windows.** GTK 2226 under Xvfb (`GDK_BACKEND=x11`), this doc's
  program (no stdin read), xdotool: main click/move/scroll = 0/0/0 events; fixed
  2/5/2. With readers (`k1015`: type `ab`, click, type `cd`, then `io::readKey(0)`):
  main `events=0 keys=abcd`, fixed `events=2 keys=abcd` (keys kept in order).
  Windows 206 headless app with the reports on stdin (`Start-Process
  -RedirectStandardInput`): main `EVENTS:` / fixed `EVENTS:[1,3][4,6]`, `CHARS:abcdef`
  both. The real-window Windows run (`bugs/repro/bug-1015/overid.*` via a `/it` task)
  could NOT run: `query session` on 206 → "No session exists" (nobody logged in at the
  console), task Last Result 267011 (never ran).
- **Not fixed (reported to the coordinator, not this bug's scope):** a CONSOLE
  (terminal) program that uses `term::pollMouse` but no `io::` reader never subscribes
  stdin (no broadcast log is built, `subscribe_stdin` is keyed on the reader calls), so
  it still sees no mouse events. The man page now states this limit.
- **Checks run.** `cargo test --release --test rt_native_term_runtime` 22 passed;
  `rt_canvas_hit` 15; `codegen_mouse_arena_region` 4; `codegen_helper_scratch_release`
  12; `app_special_keys` 37; `rt_foreign_entry_callee_saved` 6; `architecture_guards` 8;
  `no_type_strings` 7; `test_targets_registered` 4; `cargo test --release --bin mfb --
  term:: mouse:: io::input:: gen_read canvas:: builtins::tests::` 138 passed (includes
  the declared-errors sweep); `man-run-examples.sh term` 48/48, `canvas` 38/38;
  `spec-census --citations/--render/--links app` clean. Cross-builds of reader-less,
  reader and console probes for linux-aarch64/x86_64, windows-x86_64, macos-aarch64 ok.
- **Goldens expected to change** (`scripts/artifact-gate.sh <mfb> all`):
  `tests/syntax/app/app-mouse-surface/golden/app_mouse_surface.{macos-aarch64,linux-aarch64,linux-x86_64,windows-x86_64}.app.ncodesum`
  (pollMouse bodies; on top of bug-966/965) and, new with this bug,
  `app_mouse_surface.macos-aarch64.app.nplan` — localized: only added import rows
  `_read`/`___error` requiredBy `_mfb_rt_term_term_pollMouse` and
  `_mfb_rt_canvas_canvas_pollMouseRaw`. The fixture is the only one with a mouse member.
