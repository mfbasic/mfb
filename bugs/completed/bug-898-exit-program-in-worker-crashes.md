# bug-898: `EXIT PROGRAM` inside a thread worker segfaults or reports a bogus `ErrOutOfMemory`

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: FIXED (bs16-agent-a; see Resolution)
Regression Test: tests/runtime/rt_thread_exit_program_in_worker.rs

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

## Resolution

Root cause confirmed in the debugger: at `_mfb_rt_thread_finish` entry on the
worker (`bp _mfb_rt_thread_finish`, `list reg 1 x0 x1 x2 x3 x4`) the tag is `2`,
the value `7`, the message `0` and the source a stale pointer — the error arm
copied those as a `String`/`ErrorLoc`.

Fix (`lower_thread_finish`, `src/codegen/runtime/thread/runtime_helpers_thread.rs`):
after the worker's stdout drain and before the block lock, a tag-2 result calls
`_mfb_shutdown` (when the module has an entry — the new `program_shutdown`
argument, `module.entry.is_some()` in `builder/mod.rs`) and then the platform's
`emit_program_exit` with the value as the code. Only `x1` (the value) is read.
The worker's lexical cleanup already ran on the way out of its frames (bug-841's
inline-`TRAP` passthrough included) and the trampoline's teardown closed its
module-level resources. `_mfb_shutdown` never destroys the main arena in a program
that starts threads (`skip_entry_arena_destroy`), so the still-running main thread
is not pulled out from under. The main instance's module-level resource drain is
NOT run on this path: it needs the main thread's arena register, which a worker
cannot use while the main thread may be running — the same as the signal handler.
Every platform exit primitive (`_exit`, `exit_group`, `ExitProcess`) ends the whole
process. App mode: a worker's `EXIT PROGRAM` exits the process directly rather than
through the app finish helper. A release app build's finish ends in `_exit(code)`
as well (`mfb spec app macos-runtime`); a `--debug` GUI build's finish keeps the
window open by parking the CALLING thread in `pause()`, which from a worker would
leave the program thread running, so the direct exit is the only correct choice
there.

Blast radius audit: `grep -rln RESULT_ERROR_MESSAGE_REGISTER src/codegen/runtime src/target`
lists only the thread trampoline/finish and the app I/O helpers; the app helpers do
not split a user entry's result, so no other consumer reads `x2` for tag 2.

Proof: `tests/runtime/rt_thread_exit_program_in_worker.rs` (4 programs x -O0/-O3 x
10 runs): base compiler 80/80 runs wrong (SIGSEGV 68, `Error: 7-701-0001` exit 255
9, parent finished with exit 3 3); fixed 80/80 exit 7 with the expected output.
Stress of the doc repro at the default level, 210 runs each: base `149 x 139,
61 x 255`; fixed `210 x 7`. A `--debug` build exits 7 and prints its report.

Spec: `mfb spec threading thread-runtime-helpers` (§ Trampoline, the program-exit
arm) and `mfb spec language control-flow` (a worker's `EXIT PROGRAM`). Man:
`mfb man flow`.

## Golden impact

`_mfb_rt_thread_finish` gains a tag compare, a `bl _mfb_shutdown` and the exit
sequence, so the native-code goldens of every fixture that starts a thread
(`tests/rt-behavior/threads/**`, thread-using byte-identity covers) are expected
to change. No AST/IR/run-output golden changes.

### Measured

Measured with `scripts/artifact-gate.sh <mfb> all` (all five bs16-agent-a fixes built in; 59 diffs in total, attributed per function by comparing base and fixed `--ncode` for macos-aarch64): `_mfb_rt_thread_finish` is the only changed function in
`tests/byte-identity/thread/thread_codegen_cover_rt` and
`tests/byte-identity/resource-xfer-slots/resource_xfer_slots_cover_rt` (`.ncodesum`,
all five targets); bug-897 and bug-898 both change that helper, so both explain
those ten diffs. `scripts/test-accept.sh` on the thread fixtures
(`thread*`, `func_thread*`, `bug69*`, `bug630*`, `bug221*`) passes.
