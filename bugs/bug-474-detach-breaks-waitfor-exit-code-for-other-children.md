# bug-474 — `process::detach` silently destroys `process::waitFor`'s exit code for every OTHER child

- **Severity:** HIGH — a correct program silently reads a wrong exit status. No
  error, no diagnostic; a failing child reports success.
- **Status:** open
- **Found by:** plan-108 letter E cross-model review of the `process` man pages
  (the review flagged `waitFor`'s "a child that exited normally returns its
  exit status" as false after a `detach`).
- **Platforms:** Unix (macOS verified; Linux by inspection — same code path).
  Windows unaffected (no `SIGCHLD`).

## Reproduction

```mfbasic
IMPORT process
IMPORT io

SUB main()
  RES first AS process::Process = process::shell("sleep 0.1; exit 7")
  RES other AS process::Process = process::shell("sleep 5")
  process::detach(other)
  io::print("waitFor(first) = " & toString(process::waitFor(first)))
END SUB
```

```
$ ./target/release/mfb build /tmp/p && /tmp/p/build/p.out
waitFor(first) = 0        <-- WRONG, the child exited 7
```

Remove the two `other`/`detach` lines and the same program prints `7`.
`first` is never detached and is not related to `other` in any way.

## Mechanism

`process::detach` reaps by installing a **process-wide** signal disposition:

```rust
// src/codegen/builtins/process/func_detach.rs — lower_process_detach_helper_posix
// signal(SIGCHLD, SIG_IGN=1) -> kernel auto-reaps, no zombie.
abi::move_immediate(abi::c_arg(0), "Integer", sigchld),
abi::move_immediate(abi::c_arg(1), "Integer", "1"),
... emit_external_call("signal", ...)
```

`SIG_IGN` on `SIGCHLD` tells the kernel to reap **every** child of this process
immediately, not just the detached one. Any later `waitpid` therefore fails with
`ECHILD`, and `func_wait_for.rs`'s POSIX helper treats `ECHILD` as "already
reaped" and returns the handle's cached exit code — which for a child that was
never waited on is its initialised default, `0`.

So `detach` is not a per-child operation at all: it changes the exit-status
semantics of the whole program.

## Why it matters

`detach` is documented as "let a child keep running after the program exits" —
nothing about it suggests it disarms exit-status reporting for unrelated
children. A supervisor that detaches one long-running worker and then waits on
its short-lived jobs will see every one of them "succeed".

## Suggested fix

Do not touch the process-wide disposition. Reap the detached child with a
`double-fork` at spawn time, or install a `SIGCHLD` handler that reaps only
PIDs on a detached set, or set the disposition per-child by having `detach`
spawn a reaper thread that `waitpid`s just that PID. Whatever the mechanism, a
`waitFor` on a non-detached handle must keep returning the real status.

Until it is fixed, `mfb man process detach` and `mfb man process waitFor`
document the limitation explicitly (plan-108 letter E).

## Related

- bug-475 — `process::waitFor` can block forever on a child whose output nobody
  drains (found in the same review).
