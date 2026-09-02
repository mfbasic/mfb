# bug-474 — `process::detach` silently destroys `process::waitFor`'s exit code for every OTHER child

- **Severity:** HIGH — a correct program silently reads a wrong exit status. No
  error, no diagnostic; a failing child reports success.
- **Status:** FIXED
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

`process::detach` reaped by installing a **process-wide** signal disposition:

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

So `detach` was not a per-child operation at all: it changed the exit-status
semantics of the whole program.

## Why it matters

`detach` is documented as "let a child keep running after the program exits" —
nothing about it suggests it disarms exit-status reporting for unrelated
children. A supervisor that detaches one long-running worker and then waits on
its short-lived jobs will see every one of them "succeed".

## The fix

The process-wide disposition is gone. `detach` now reaps the one child it
detached on a dedicated thread:

- `_mfb_rt_process_reaper` (`gen_unix.rs:lower_process_reaper_helper`) — a
  pthread start routine that does `waitpid(pid, NULL, 0)` on exactly that pid
  and returns. The pid is passed **by value**, never a pointer to the `Process`
  record, whose arena block is reclaimed when the detaching scope exits while
  the thread may still be blocked in `waitpid`.
- `detach` calls `pthread_create` + `pthread_detach`, checking the create
  return so a failed create never hands garbage to `pthread_detach`. At the
  thread limit `pthread_create` returns `EAGAIN`, the detach simply skips the
  reaper, and the child is left for the program's exit to reparent — `detach`
  still succeeds and no other child's status is touched.
- The reaper retries on `EINTR`: signal delivery is process-wide, so any signal
  the program takes while the thread sits in `waitpid` would otherwise return
  without reaping and leave exactly the zombie the thread exists to prevent.
- `builder/mod.rs` emits the reaper whenever `process.detach` is present and the
  platform is not Windows; `linux_common/plan.rs` and `macos_aarch64/plan.rs`
  pull `pthread_create`/`pthread_detach` for `process.detach` only, so the rest
  of the package stays libc-only.

**A third sub-issue found while fixing, not in the original report:** detaching a
handle a `waitFor` (or `isRunning`) had *already reaped* must start no reaper at
all. That pid may already have been recycled onto a later child of ours, and the
reaper would then consume *that* child's exit status — bug-474 in miniature.
`detach` now branches on `PROC_REAPED` and skips the thread entirely.

Audited all five `waitpid` emission sites (`func_wait_for.rs:145`,
`func_is_running.rs:138`, `gen_unix.rs:108,629,969`): every one passes a specific
pid, never `-1`, so the reaper thread cannot steal another child's status.

The false limitation warnings this bug forced onto `mfb man process detach` and
`mfb man process waitFor` (plan-108 letter E) are removed, and `detach`'s page
now states the opposite and true fact: detaching one child affects only that
child.

## Verification

RED→GREEN proven on byte-identical source, same machine, only the compiler
differing. The regression test is `tests/rt_process_detach_preserves_exit_code.rs`,
whose three assertions are the bug (`first=7`), `detach`'s other contract that
must not be traded away (`probe=.`, no zombie), and the recycled-pid sub-issue
(`done=3`/`after=5`).

| Gate | Result |
| --- | --- |
| Doc repro at `955ae8779` (unfixed) | `waitFor(first) = 0` — bug reproduced, mechanism confirmed |
| Doc repro on fix | `waitFor(first) = 7` |
| Test source on unfixed `main` | `first=0 probe=. done=0 after=0` — RED on the two bug assertions, guard passes |
| Test source on fix | `first=7 probe=. done=3 after=5` |
| `cargo test --release --test rt_process_detach_preserves_exit_code` | `ok. 1 passed` |
| Full `cargo test --release --no-fail-fast` | 88 suites, **4363 passed, 0 real failures** |
| `cargo test --release --test golden` (uncontended) | `1325 tests, 1823 golden(s), 0 diff(s)` |
| `scripts/artifact-gate.sh target/release/mfb all` | `1325 tests, 1823 golden(s), 0 diff(s)` |
| `scripts/test-accept.sh target/release/mfb /tmp/b474-accept` | `acceptance tests passed (1346 test(s) ran)`, 0 mismatches |
| linux-aarch64 glibc (box 2223) | `first=7 probe=. done=3 after=5` |
| linux-x86_64 glibc (box 2228) | pass — and RED there pre-fix (`first=0 done=0 after=0`) |
| linux-x86_64 musl (box 2227) | pass |
| linux-riscv64 musl (box 2229) | pass |
| windows-x86_64 (box 2230) | `first=7 done=3 after=5` — unaffected, as the report predicted |

Boxes 2224 (aarch64 musl) and 2232 (riscv64 glibc) were down; the surviving
matrix still covers all three architectures and both libc worlds.

`.ncodesum` drift: 4 unix targets regenerated (`process_codegen_cover_rt` for
macos-aarch64, linux-aarch64, linux-x86_64, linux-riscv64). There is no
`windows-x86_64.ncodesum` for `process` (`process.shell` is unsupported there),
which is why Windows was proven live on box 2230 rather than by byte-identity.

## Deviations

- The original report's "Suggested fix" offered double-fork, a `SIGCHLD`
  handler, or a per-child reaper thread. The reaper thread was taken: the other
  two both need a process-wide signal disposition or a fixed-capacity global
  table, which is what made this a whole-program defect in the first place. The
  cost is one thread per *live* detached child, each exiting the moment its
  child does.
- Sub-issue 3 (already-reaped handle) was not in the report; it was found while
  fixing and is covered by the same test.

## STATUS: FIXED (5bdab074a)

Landed by 5bdab074a (reaper thread replaces `SIGCHLD=SIG_IGN`), 7176456be
(`EINTR` retry + pthread-entry trap notes), e3d073a1c (4 unix `.ncodesum`
regenerations), 54bd7dcf2 (skip the reaper when already reaped).

## Related

- bug-475 — `process::waitFor` can block forever on a child whose output nobody
  drains (found in the same review).
