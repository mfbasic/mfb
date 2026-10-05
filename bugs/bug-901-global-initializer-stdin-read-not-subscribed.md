# bug-901: a module-level initializer that reads stdin fails with `ErrInvalidContext` — the main thread is subscribed only after global initialization

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/io/global-init-reads-stdin-rt (proposed)

`LET FIRST AS String = io::readLine()` at module level makes a single-threaded
program fail at startup. It prints `Error: 7-705-0019 Operation was invoked from a
thread that is not permitted to perform it (e.g. reading stdin from a thread that
has not called thread::openStdIn).` and exits 255. The program never calls
`thread::` at all, and nothing in the error tells the user that the cause is
reading stdin in a global initializer.

**Correct behavior:** the main thread can read stdin from the first user code that
runs, global initializers included. The repro below prints `first=[one]` and
`second=[two]`.

References:

- `src/docs/spec/threading/09_os-integration.md` § Standard Input Broadcast: "The
  compiler subscribes the **main** thread at program entry whenever the module uses
  a stdin builtin, so a single-threaded program is byte-identical to a direct
  per-byte reader with no source change."
- `src/docs/spec/memory/08_program-startup.md` § Running the Program, steps 3–4. The
  spec writes down the faulty order: "Stdin subscription. When required, subscribe the
  main thread ... after global initialization and before the entry call." That
  sentence needs to change together with the code.
- Found during spec hardening batch B09 (program startup).

## Failing Reproduction

```
mfb init nv
# nv/src/main.mfb
IMPORT io

LET FIRST AS String = io::readLine()

FUNC main AS Integer
  io::print("first=[" & FIRST & "]")
  LET second AS String = io::readLine()
  io::print("second=[" & second & "]")
  RETURN 0
END FUNC

mfb build nv -q && printf 'one\ntwo\n' | ./nv/build/nv.out; echo "exit=$?"
```

- Observed:
  ```
  Error: 7-705-0019
  Operation was invoked from a thread that is not permitted to perform it (e.g. reading stdin from a thread that has not called `thread::openStdIn`).
  exit=255
  ```
- Expected: `first=[one]`, `second=[two]`, `exit=0`.

Contrast: the same `io::readLine()` inside `main` works. In a worker the trampoline
also runs the global initializer, and a worker is unsubscribed until
`thread::openStdIn`, so there `ErrInvalidContext` is the documented result.

## Root Cause

`src/codegen/engine/function/entry.rs:lower_program_entry` emits the
`_mfb_rt_stdin_subscribe` call after the `LINK` init and the global initializer.
The code comment gives the reason: "Runs after the global/link initializers (so the
log's lazy setup is single-threaded)". Both initializers already run on the main
thread before `main`, so subscribing before them is just as single-threaded.

## Goal

- Subscribe the main thread before the `LINK` and global initializers (right after
  the arena is live), and update step order 2–4 in spec 08.
- The repro prints `first=[one]` / `second=[two]`.

### Non-goals (must NOT change)

- Workers stay unsubscribed until `thread::openStdIn`.
- Programs that do not use stdin builtins must not change: no subscribe call is
  emitted for them.

## Blast Radius

- Main-thread console entry: fixed here.
- The app-mode program symbol (the same `lower_program_entry` run on a worker under
  the app bootstrap) gets the same reorder.
