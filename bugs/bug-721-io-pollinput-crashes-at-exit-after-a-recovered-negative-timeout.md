# bug-721: after a recovered `io::pollInput(-1)` failure, a later successful `pollInput` crashes the program at exit (SIGBUS)

Last updated: 2026-09-16
Effort: medium
Severity: HIGH
Class: Correctness (process crash; memory-safety symptom)

Status: Open
Regression Test: none yet — see Phase 1

`io::pollInput` with a negative timeout correctly raises `ErrInvalidArgument`
(`77050002`), and an inline `TRAP` recovers from it. But if the program then calls
`io::pollInput` again **successfully**, the process dies with signal 10 (SIGBUS)
when it exits. All of its output is printed first, so the crash is at teardown.
It happens with a regular file on standard input and with no input at all.

**The single correct behavior a fix produces:** a recovered `pollInput` failure
leaves no state behind. A later `pollInput` call and program exit behave exactly as
if the failed call had never happened, and the process exits 0.

Found by plan-125-C/D (letter D Phase 1) while verifying the `io::pollInput` man
page's negative-timeout claim. **Filed, not fixed**, by user instruction during a
documentation-only plan ("file all bugs, make no fixes").

## Reproduction

```basic
IMPORT io

SUB main()
  LET a = toString(io::pollInput(-1)) TRAP(e)
    RECOVER "<raised " & toString(e.code) & ">"
  END TRAP
  io::print("E pollInput(-1) = " & a)
  LET b = toString(io::pollInput(0)) TRAP(e2)
    RECOVER "<raised " & toString(e2.code) & ">"
  END TRAP
  io::print("E pollInput(0) = " & b)
END SUB
```

Observed (macos-aarch64, `worktree-P-125`, `/tmp/p125-ex/iopollE`), both with a byte
file on standard input and with standard input from `/dev/null`:

```
E pollInput(-1) = <raised 77050002>
E pollInput(0) = TRUE
                                   <- process exits with signal 10 (SIGBUS), exit -10
```

Expected: the same two lines, then exit 0.

The variants that isolate it (all with the same input file):

| Probe | Sequence | Result |
|---|---|---|
| `iopollA` | `pollInput(0)` only | exit 0 |
| `iopollF` | `pollInput(0)`, `pollInput(0)` | exit 0 |
| `iopollD` | trapped `pollInput(-1)`, then a plain `io::print` | exit 0 |
| `iopollG` | trapped `pollInput(-1)`, then `io::readChar` | exit 0 |
| `iopollE` | trapped `pollInput(-1)`, then a successful `pollInput(0)` | **SIGBUS at exit** |
| `iopollB` | the same as E, inside a helper `FUNC` | **SIGBUS** (before the second line prints) |

So neither a failed poll nor two polls crash on their own; the failed poll followed by
a successful one does.

## Root cause

Hypothesis, to confirm in Phase 1. The negative-timeout check raises after some
per-call poll state has been set up (a registration, an allocation, or a waiter
record for the thread's standard-input view). The error path returns without undoing
it. The next successful `pollInput` reuses or re-registers that state, and the
duplicate or dangling record is freed twice, or touched after release, when
standard input is torn down at exit.

Confirm by reading the `pollInput` lowering and the standard-input teardown
(`src/codegen/io/stdin/stdin_broadcast.rs`) for state created before the timeout
validation, and by running the reproduction under a debugger to get the faulting
address at exit.

## Non-goals

- Changing `pollInput`'s behavior for valid timeouts or its `ErrInvalidArgument` for
  negative ones.
- Masking the crash by skipping standard-input teardown at exit.

## Blast-radius audit

- Other `io` calls that can raise after touching standard-input state: `readLine`,
  `readChar`, `readByte`, `input` (their `ErrEncoding` and `ErrInvalidContext` paths),
  each followed by a successful call of the same kind.
- `thread::openStdIn`/`closeStdIn` around a failed poll in a worker thread.

## Fix

Phase 1 — RED test from the reproduction (expect exit 0 and both lines), plus the
audit variants. Commit:

Phase 2 — move the timeout validation before any state is created, or undo that
state on the error path (GREEN); full suite, and the `io` runtime goldens. Commit:
