# bug-898: `EXIT PROGRAM` inside a thread worker segfaults or reports a bogus `ErrOutOfMemory`

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: tests/rt-behavior/threads/thread-exit-program-in-worker-rt (proposed)

When a worker entry function (or any function it calls) runs `EXIT PROGRAM n`, the
process does not exit with code `n`. Instead it nondeterministically dies with
SIGSEGV (exit 139) or prints `Error: 7-701-0001` / `Allocation failed.` and exits
255. It happens at -O0, -O1 and -O3 and whether or not the parent ever calls
`thread::waitFor`.

**Correct behavior:** `EXIT PROGRAM n` "terminates the process with the given exit
code from any call depth" (language spec 10), so a worker's `EXIT PROGRAM 7` must
end the process with exit status 7 (after its lexical cleanup), never touch the
exit-code register as if it were an error message, and never crash.

References:

- `src/docs/spec/language/10_control-flow.md`: "`EXIT PROGRAM <integer>` terminates
  the process with the given exit code from any call depth ... it propagates past
  every `TRAP` to the entry point."
- `src/docs/spec/memory/02_fallible-call-abi.md`: tag `2` = program exit; "The
  program-exit tag is checked before the success/error split".
- `src/docs/spec/threading/06_thread-runtime-helpers.md` § Trampoline: "Captures the
  returned `Result OF Out` — tag, value, error message, and error source" — silent
  on tag 2; `_mfb_rt_thread_finish` "a failure's message (`String`) ... through their
  fixed copy functions".
- Related but distinct: bug-841 (EXIT PROGRAM under an inline TRAP on the main
  thread). Found during spec hardening batch B09 (threading).

## Failing Reproduction

```
mfb init t6b
# t6b/src/main.mfb:
IMPORT io
IMPORT thread

ISOLATED FUNC quitter(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  IF n > 0 THEN EXIT PROGRAM n
  RETURN 0
END FUNC

FUNC main AS Integer
  RES a AS Thread OF Integer TO Integer = thread::start(quitter, 7)
  LET r AS Integer = thread::waitFor(a)
  io::print("after waitFor r=" & toString(r))
  RETURN 3
END FUNC

for O in 0 1 3; do mfb build t6b -q -O$O; for i in 1 2 3 4 5; do ./t6b/build/t6b.out; echo "O$O exit=$?"; done; done
```

- Observed (15 runs, macOS AArch64): `exit=139` 9 times; the other 6 print
  ```
  Error: 7-701-0001
  Allocation failed.
  ```
  and `exit=255`.
- Expected: no output, `exit=7`, every run.

Variant: with `os::sleep(300)` before `waitFor` in main, the process still dies
with 139 before the sleep ends (nothing printed), so the crash is in the worker's
exit path, not in `waitFor`.

Contrast: `FAIL error(...)` in a worker, an integer divide by zero in a worker, and a
normal return all cross correctly (`thread::waitFor` reports code, message and
`file:line:char`).

## Root Cause

`src/codegen/runtime/thread/runtime_helpers_thread.rs:lower_thread_finish` splits
the parked result only on `tag == RESULT_OK_TAG`; every other tag goes to the
`error_copy` arm, which treats `x2` as a `String` message and `x3` as an
`ErrorLoc` and deep-copies both into the block arena via the fixed
`_mfb_rt_thread_copy_value_String_*` / ErrorLoc copy functions. For
`RESULT_PROGRAM_EXIT_TAG` (`2`) `x1` is the exit code and `x2`/`x3` are not set
(stale register contents), so the copy dereferences garbage: a wild length makes
`_mfb_arena_alloc` fail (the `copy_failed` → `ErrOutOfMemory` result, which the
parent's `waitFor` then propagates to the entry → `Error: 7-701-0001`, exit 255)
or the load faults (SIGSEGV). Neither the trampoline
(`src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_trampoline`) nor
`lower_thread_finish` has a program-exit arm (`grep -rn PROGRAM_EXIT
src/codegen/runtime/thread/` is empty).

## Goal

- A worker whose entry returns tag 2 terminates the process with `x1` as the exit
  code (through the same shutdown path the main entry's `entry_exit` uses), and no
  register other than `x1` is read as a value.
- The threading spec (§ Trampoline) states the program-exit arm.

### Non-goals (must NOT change)

- Tag 1/3 error transport across `waitFor` (message, code, `ErrorLoc`) — currently
  correct.
- Do not "fix" by mapping tag 2 to an error stored for `waitFor`; `EXIT PROGRAM` is
  not catchable.

## Blast Radius

- `lower_thread_finish` — fixed by this bug.
- Any other consumer that branches on `tag != OK` alone and then reads `x2` as a
  message (e.g. graphics/app worker trampolines) — not audited here; grep
  `branch_ne(&error_copy)`-style splits in `src/codegen/runtime/`.
