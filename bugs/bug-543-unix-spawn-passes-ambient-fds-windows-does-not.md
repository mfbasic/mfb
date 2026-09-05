# bug-543: `process::spawn` hands the child ambient inherited fds on Unix, but not on Windows

Last updated: 2026-09-04
Effort: medium (3h–1d)
Severity: LOW–MEDIUM (defense-in-depth; a platform-inconsistent security contract)
Class: security / cross-platform consistency

Status: OPEN — needs a decision before it is worth implementing (see "The
decision" below). The CI failure it produced is fixed in the test harness; this
doc is about the product asymmetry that failure exposed.

## The finding

bug-499 gave `process::spawn` two different guarantees on two platforms:

* **Windows** — `bInheritHandles = FALSE` plus a `STARTUPINFOEXA`
  `PROC_THREAD_ATTRIBUTE_LIST` naming the three stdio handles. That is an
  **exhaustive, process-side gate**: the child receives those three handles and
  nothing else, no matter what the parent itself was handed.
* **Unix** (`src/codegen/builtins/process/gen_unix.rs`) — per-descriptor
  close-on-exec: `pipe2(O_CLOEXEC)` / `pipe` + `fcntl(F_SETFD, FD_CLOEXEC)`,
  `O_CLOEXEC` on `fs::open`, `SOCK_CLOEXEC` on sockets. That covers **every
  descriptor MFBASIC opens** — and only those.

So on Unix a descriptor the MFB program's own *launcher* left inheritable passes
straight through `process::spawn` into the child. MFBASIC never opened it, so no
CLOEXEC flag was ever set on it, and nothing in the spawn path closes it.

## Reproduction (macOS, 2026-09-04, release binary)

`fdprobe.c` is the probe from `tests/rt_process_spawn_no_fd_inherit.rs`: it
`fstat`s every fd from 3 up and prints what it finds. The MFB parent opens a file
and a TCP listener, then spawns it.

```sh
# clean shell
$ ./fdexp.out
leaked=none
exit=0

# same binary, two inheritable fifos handed in by the launcher
$ mkfifo /tmp/f1 /tmp/f2
$ exec 142<>/tmp/f1; exec 145<>/tmp/f2
$ ./fdexp.out
leaked=142:fifo,145:fifo
exit=0
```

The second run is not hypothetical: it is character-for-character what the
GitHub Actions Linux and macOS runners produced
(https://github.com/mfbasic/mfb/actions/runs/33943384178) — the runner leaks two
non-CLOEXEC pipes, they descend runner → shell → cargo → test binary → MFB
program → spawned child, and the probe sees them. The same program on Windows
would show none of this, because the inheritance list there is exhaustive.

## The decision

Two defensible readings, and they are materially different work:

1. **Unix should match Windows.** `tests/rt_process_spawn_no_fd_inherit.rs`'s
   own header states the strong contract — "only the three stdio pipes the spawn
   deliberately hands over" — and MFB exposes no way to pass a descriptor to a
   child, so nothing legitimate is lost by closing the rest. Implementation: in
   the forked child, after the `dup2` dance and before `execvp`, close every fd
   above 2 **except the self-pipe write end** (which must stay open, and stay
   CLOEXEC, to carry `errno` on exec failure). Cheapest shape is to `dup2` that
   end onto fd 3, re-set `FD_CLOEXEC` on it, then close from 4 up. The close
   itself wants `close_range(4, ~0u, 0)` on Linux (syscall 436, 5.9+) with a
   `getdtablesize()` loop as the fallback, and the loop on macOS. Touches four
   Unix targets and moves the `process` goldens on all of them.
2. **Unix is already correct.** "The runtime closes what the runtime opens" is
   what Go's `os/exec` and Rust's `std::process` do; an inheritable descriptor in
   the parent is the launcher's bug. Under this reading Windows is simply
   stricter because the platform made it free, and the asymmetry is documented
   rather than removed.

Python (`subprocess`, `close_fds=True` since 3.2) and Ruby (`Process.spawn`) both
chose (1), and both chose it on security grounds.

Whichever is chosen, `mfb man process spawn` and `mfb spec` should state what a
child inherits — today neither says, which is why this went unnoticed.

## What is already fixed

Not this. The CI redness it caused is fixed in the harness only:
`common::run_bounded_without_inherited_fds` closes the non-CLOEXEC descriptors
this test process was handed, in the forked child before `exec`, so the probe
measures what MFBASIC leaked rather than what the runner leaked. The
`leaked=none` assertion is unchanged. Verified RED (`leaked=142:fifo,145:fifo`)
then GREEN under a shell holding the same two fifos.
