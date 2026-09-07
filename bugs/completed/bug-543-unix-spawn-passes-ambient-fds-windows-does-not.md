# bug-543: `process::spawn` hands the child ambient inherited fds on Unix, but not on Windows

Last updated: 2026-09-06
Effort: medium (3h–1d)
Severity: LOW–MEDIUM (defense-in-depth; a platform-inconsistent security contract)
Class: security / cross-platform consistency

Status: **FIXED** (2026-09-06, `5eb765a58`). The guarantee is now the same on all
three platforms. Linux sweeps with `close_range`; macOS execs through
`posix_spawn` with `POSIX_SPAWN_CLOEXEC_DEFAULT`; Windows was already exhaustive
and is unchanged. See "What was actually done" at the bottom, including three
places this document was wrong.


## USER DECISION (2026-09-06) — the mechanism, not just the guarantee

The guarantee was already ruled on: **the same on all platforms.** The remaining
question was how macOS gets there, and the ruling is
**`posix_spawn` + `POSIX_SPAWN_CLOEXEC_DEFAULT`**.

That makes the guarantee STRUCTURAL rather than a scan — everything not named in
the file-actions is closed by the spawn itself, which is the same shape Windows
already has. The rejected alternative (a `/dev/fd` readdir in the fork child) was
smaller but keeps the guarantee as an enumeration, and `readdir` in a fork child
is not async-signal-safe.

Consequences a fix must handle, both of which follow from leaving fork/exec:

- The macOS spawn path is REWRITTEN off `fork`/`exec`, not patched.
- **An ignored signal disposition survives `exec`** — the reset to `SIG_DFL`
  currently done between fork and exec has no "between" any more. It has to move
  into the `posix_spawn` attributes (`POSIX_SPAWN_SETSIGDEF` with the full set),
  or a spawned child silently inherits an ignored `SIGPIPE`/`SIGINT`. This is
  the trap that will be missed; a spawned child that never dies on a closed pipe
  is the symptom.
- The doc's originally suggested close-loop is DISPROVED and must not be
  restored: `getdtablesize()` measured 245,760 on this host, so the loop is a
  quarter-million syscalls per spawn.

Linux is settled independently: `close_range`.

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

## The mechanism question (2026-09-05) — measured, and it reopens the design

**RED reproduced** on this tree, macOS aarch64 release, with a child that lists
its own descriptors (`sh -c 'ls /dev/fd'`):

    clean shell        childfds=0 1 2 3 4
    two ambient fifos  childfds=0 1 142 145 2 3 4

The delta is `142,145` — descriptors MFBASIC never opened, passing straight
through `process::spawn`. (3 and 4 belong to the probe's own `ls | tr` pipeline,
which is why the delta is the finding rather than the absolute set.)

The contract this violates is written down in-tree, not merely implied:
`tests/rt_process_spawn_no_fd_inherit.rs`'s header says the child must
"inherit … only the three stdio pipes" and "must see NO open fd above 2".

**But this document's proposed fallback is not viable, measured:**

    $ getdtablesize()   ->  245760      # on this machine
    $ closefrom(4)      ->  error: call to undeclared function 'closefrom'

So "a `getdtablesize()` loop as the fallback, and the loop on macOS" would cost
~245,000 `close` syscalls **per spawn**, and macOS ships no `closefrom` to
replace it. The Linux half is fine — `close_range(4, ~0u, 0)` (syscall 436, 5.9+)
is one syscall — so the platforms need different mechanisms and only one of them
is settled.

### The three macOS candidates

1. **`posix_spawn` with `POSIX_SPAWN_CLOEXEC_DEFAULT`.** The correct answer, and
   the exact analogue of what Windows already does — an exhaustive, kernel-side
   gate rather than a per-descriptor sweep. Cost: it replaces fork+exec on the
   macOS path, so the child-side `cwd`/env setup moves into
   `posix_spawn_file_actions`/`posix_spawnattr`, and the self-pipe errno protocol
   is replaced by `posix_spawn`'s own return. That is a rewrite of
   `emit_spawn_tail`'s macOS half, not an insertion into it.
2. **Scan `/dev/fd` in the fork child.** Correct and cheap at run time, but
   `opendir`/`readdir` allocate, and the child deliberately does not allocate
   (see `emit_spawn_tail`'s own note). Doable with `getdirentries` into a stack
   buffer; fiddly in emitted assembly.
3. **The loop.** Disproved above.

None of these is a coin-flip, so the mechanism is worth deciding before anyone
starts: (1) is the most correct and the most work; (2) keeps the existing
structure. The Linux half can land independently either way — it is one syscall
and needs no decision.

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


## What was actually done (2026-09-06)

### The fix

* **Linux** (`gen_unix.rs::emit_spawn_tail`, child, after the `dup2` dance): the
  self-pipe write end is `dup2`'d onto fd 3 and re-marked `FD_CLOEXEC`, then
  `close_range(4, ~0u, 0)` sweeps everything above it. One syscall.
* **macOS**: the child execs through
  `posix_spawnp(NULL, argv[0], &fa, &attr, argv, environ)` with
  `POSIX_SPAWN_CLOEXEC_DEFAULT | POSIX_SPAWN_SETSIGDEF | POSIX_SPAWN_SETEXEC`.
  The file-actions are three `adddup2(fd, fd)` entries for 0/1/2, so the kernel
  hands the new image those three descriptors and nothing else.
* **Windows**: verified, not rewritten.
* `mfb man process spawn` and `mfb man process shell` now state what a child
  inherits. There is no `process` chapter under `mfb spec` at all (the `stdlib`
  spec covers regex/datetime/csv/json/http/url/math-rng/encoding/vector/audio/
  bits/money/os/astrings/icmp/transports/color and no others), so the man pages
  are the whole doc surface for this guarantee.

### Three corrections to this document

1. **"Windows — `bInheritHandles = FALSE`" is wrong.** `gen_windows.rs` passes
   `bInheritHandles = **TRUE**` and gets its exhaustiveness from the
   `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` in the `STARTUPINFOEXA`. That is not a
   nit: with `FALSE` the handle list is inert and the child would receive no
   handles at all. The *conclusion* the document draws from it — that the
   Windows gate is exhaustive — is correct.
2. **The macOS rewrite is smaller than the document predicted.** It says the
   `posix_spawn` route "replaces fork+exec on the macOS path, so the child-side
   `cwd`/env setup moves into `posix_spawn_file_actions`/`posix_spawnattr`, and
   the self-pipe errno protocol is replaced by `posix_spawn`'s own return".
   With `POSIX_SPAWN_SETEXEC` — `posix_spawn` behaving as an exec of the calling
   image rather than as a fork — the fork stays, so `chdir`/`setenv` stay exactly
   where they were (no `posix_spawn_file_actions_addchdir_np`, no hand-built
   `envp`) and the self-pipe protocol stays (it now carries `posix_spawn`'s
   return value, which *is* the error number, instead of `errno`). The
   kernel-side descriptor gate is identical either way, which is the part that
   was ruled on. `posix_spawnp` was measured to match `execvp` on both PATH
   search and the `ENOEXEC`-to-`sh` fallback for a shebang-less script.
3. **`close_range` could not have been a libc import.** musl 1.2.6 — both Alpine
   boxes — exports no `close_range` wrapper at all (`nm -D` on
   `libc.musl-*.so.1`: nothing; the header declaration is missing too), so a
   `close_range` PLT import would not link there. It goes out as raw syscall
   **436**, which is the number on x86-64, AArch64 and RISC-V alike, and that
   also makes the glibc-2.34 floor irrelevant. A kernel older than 5.9 answers
   `-ENOSYS`; a bounded `close(4..1024)` loop is emitted behind that check and
   was proven live by forcing `close_range` to fail with an invalid flags word.

### The trap, measured

Dropping `POSIX_SPAWN_SETSIGDEF` from the flags (`16452` → `16448`) and
rebuilding turns two of the new tests red on macOS: the child reads back
`sigpipe=ignored`, and a spawned `writer | head -c 8` pipeline never terminates —
the 30s bound fires. That is the failure this change would have introduced if the
signal reset had simply been deleted along with the fork child that used to hold
it.

### Proof

`tests/rt_process_spawn_ambient_fds.rs` (4 cases: ambient-fd, stdio positive pin,
signal disposition, closed-pipe pipeline). RED before the fix with
`leaked=142:file,145:file`; green after, on macOS-aarch64 **and** — through
cross-compiled binaries shipped and executed — on Linux x86_64/glibc (2228),
x86_64/musl (2227), aarch64/glibc (2223) and riscv64/musl (2229). Windows was
compile-tested only, as everything Windows in this repo is.
