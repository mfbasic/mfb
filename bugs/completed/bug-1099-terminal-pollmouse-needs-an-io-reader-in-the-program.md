# bug-1099: in a terminal, `term::pollMouse` sees no events unless the program contains an `io::` reader, even one that never runs

Last updated: 2026-10-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: `rt_native_term_runtime::native_term_poll_mouse_needs_no_io_reader_in_the_program`

## Cause (coordinator, bug-staging-24)

- Rule: `pollMouse` returns the events the surface received whether or not the program reads standard input (owner decision on bug-1015); in a terminal the surface's reports arrive on stdin.
- Where: a console build subscribes its main thread to stdin (plan-15) and reserves the input buffer (`src/codegen/io/input/mod.rs:program_uses_input_buffer`, `INPUT_BUFFER_READERS`) only when an `io::` reader is in the program, so `emit_decode_pending_input` (`gen_read_family.rs`) skips a program that only polls the mouse.
- Fix: a program that uses the mouse readers (`term::pollMouse`/`canvas::pollMouse`, the runtime symbols `term::enableMouse` brings in) is a stdin reader for that decision: it subscribes and reserves the buffer like an `io::` reader, and the drain then runs. Deletes the "a terminal program with none sees no mouse events" sentence bug-1015 added to `mfb man term pollMouse`.

Found by bs24-agent-b while fixing bug-1015 (which made `pollMouse` decode pending input
itself), reproduced by the bug-staging-24 coordinator, and placed in a later phase.
bug-1015's fix documented this as a limit in `mfb man term pollMouse` ("a terminal program
with none sees no mouse events"); that sentence goes when this is fixed.

The owner's bug-1015 rule is that `pollMouse` returns the events the surface received
whether or not the program reads standard input. In a console (terminal) build the
main thread subscribes to standard input only when the program uses an `io::` reader
(plan-15), so a program that polls the mouse and reads nothing has no input to decode.

**The single correct behavior a fix produces:** the program below reports `events=2`
whether or not it contains an `io::` reader.

## Reproduction (bug-staging-24 `40103c04e`, which has bug-1015)

```
IMPORT term
IMPORT io
IMPORT os

FUNC main() AS Nothing
  term::enableMouse(TRUE)
  MUT seen AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 20
    LET e AS term::MouseEvent = term::pollMouse()
    IF e.kind <> term::MouseKind.None THEN
      seen = seen + 1
    END IF
    os::sleep(20)
    i = i + 1
  END WHILE
  io::printError("events=" & toString(seen))
END FUNC
```

```
mfb build -q .
(printf '\033[<0;5;5M\033[<0;5;5m'; sleep 1) | ./build/<name>.out
```

- Observed: `events=0`.
- With `IF seen < 0 THEN LET k AS String = io::readLine() END IF` added before the last
  line (never executed): `events=2`.

## Where

`src/codegen/builtins/io/gen_read_family.rs:emit_decode_pending_input` skips a console
build without the input buffer, because such a program's main thread is not subscribed
to standard input; the subscription is decided from the program's runtime symbols
(`src/codegen/io/input/mod.rs:program_uses_input_buffer` and the plan-15 stdin
subscription). `term::enableMouse` turns reporting on without making the program a
stdin reader.

## Fix

A mouse poll is now a stdin reader for every decision that an `io::` reader drives, from ONE
source each: `src/codegen/io/input/mod.rs` gains `program_reads_mouse` (runtime symbols ending
`_pollMouse`/`_pollMouseRaw`), which `program_uses_input_buffer` now includes (the input-buffer
reservation and the decoder), and `STDIN_READER_CALLS` (with `term.pollMouse`), which the
builder uses for the main-thread stdin subscription (`subscribe_stdin`) and the stdin broadcast
log data object; the stdin helper functions (`uses_stdin`) key on `program_reads_mouse`.
`term.pollMouse` also pulls the stdin-broadcast libc imports on macOS/Linux and the reader
import set on Windows (`src/target/*/plan.rs`). The "a terminal program with none sees no
mouse events" sentence is gone from `mfb man term pollMouse` and the specs say the new rule.

RED before / GREEN after (runs the repro program, 2 SGR reports on stdin, expects `EVENTS:2`;
before: `EVENTS:0`): `cargo test --release --test rt_native_term_runtime -- needs_no_io_reader`.

Goldens expected to change: every fixture that calls `term::pollMouse` in a console or `--app`
build (the arena entry frame gains the input-buffer region and the byte-identity/native
artifacts gain the stdin log data object, stdin helpers, decoder and libc imports) --
`tests/syntax/app/app-mouse-surface` and any acceptance/byte-identity program using
`term::pollMouse`; programs without a mouse poll are unchanged. `canvas::pollMouse` programs
(always `--app`) are unaffected by the stdin helpers but gain the input-buffer region
(`program_uses_input_buffer` keys on `_pollMouseRaw`).

Pin corrected (only the disproved line): `tests/codegen/codegen_mouse_arena_region.rs`
`mouse_region_is_appended_and_moves_no_existing_arena_offset` asserted a mouse program adds
exactly `MOUSE_STATE_SLOTS`. It protects "regions are appended, no existing offset moves"
(plan-94-A, bug-690); only it depends on the count. Proof it was wrong: the owner rule in this
bug's Cause note makes a mouse poll a stdin reader, and an `io::readLine` program reserves
`INPUT_STATE_SLOTS`; the test now expects `MOUSE_STATE_SLOTS + INPUT_STATE_SLOTS` (the prefix
and frame-growth claims are unchanged). Ran: `cargo test --release --test
codegen_mouse_arena_region` (4 passed) and `bash scripts/test-fast.sh --test
rt_native_term_runtime --test codegen_mouse_arena_region` (only that pin failed, then fixed).
