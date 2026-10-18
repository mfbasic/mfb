# bug-1099: in a terminal, `term::pollMouse` sees no events unless the program contains an `io::` reader, even one that never runs

Last updated: 2026-10-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: none yet

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
