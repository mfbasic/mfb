# bug-499: spawned children inherit open fds/sockets — `fs::open`/socket lack CLOEXEC (unix), `CreateProcessA` uses `bInheritHandles=TRUE` (Windows)

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (privilege/secret leak across a process boundary)

Status: FIXED (1e554b2b3 fix, 8db76a84b goldens)

Verified RED then GREEN against a built compiler: `tests/rt_process_spawn_no_fd_inherit.rs`
builds a small C probe that `fstat`s every fd from 3 up. Reverting only `src/`
and rebuilding, the child reports `leaked=3:file,4:socket` (an open file AND a
TCP listener crossed `execvp`); with the fix it reports `leaked=none`. The
positive half — the child still receives its intended stdin/stdout/stderr and
exit code — passes in BOTH builds, which is the pin against a fix that closes
too much (e.g. setting CLOEXEC on a pipe end after `dup2`ing it onto 0/1/2).

Windows is fixed by a different mechanism, deliberately: `bInheritHandles=FALSE`
plus a `STARTUPINFOEXA` `PROC_THREAD_ATTRIBUTE_LIST` naming the three stdio
handles as the only inheritable ones. That is a process-side, exhaustive gate,
so file and socket creation need no per-handle flag there — which is why the
windows-x86_64 goldens for fs/tcp/udp/tls/net did NOT move while `process` moved
on all five targets.

Gates: artifact-gate all 0 diffs (36 goldens re-synced across the nine
fd-touching fixtures; the other 105 of 141 byte-identical); cargo test
--no-fail-fast exit 0, 119 test binaries.

Regression Test: an rt fixture that opens a file/socket, spawns a child that lists its own fds, and asserts the fd is absent.

## Summary

A child spawned by `process::spawn`/`process::shell` inherits every file
descriptor and socket the parent had open, because MFBASIC opens them without the
close-on-exec flag. On Unix `fs::open` and the socket helpers omit
`O_CLOEXEC`/`SOCK_CLOEXEC`; on Windows `CreateProcessA` is called with
`bInheritHandles = TRUE` and no explicit handle list. A spawned helper — often a
less-trusted program — therefore receives the parent's open secret files, its
listening/connected sockets (including a TLS socket's fd), and pipe ends.

## Mechanism

`fs::open`'s Linux flag set has no `O_CLOEXEC` (0x80000):

```rust
// src/codegen/builtins/fs/gen_open.rs:32 (Linux, no-follow=false)
read: "0", write: "577", read_write: "66", append: "1089",   // no 0x80000 bit
```

Sockets are created bare (`src/codegen/os/socket/shared.rs:141`):
`SOCK_STREAM = "1"`, `SOCK_DGRAM = "2"` — no `SOCK_CLOEXEC` OR'd in.

Windows (`src/codegen/builtins/process/gen_windows.rs:1028`):
`store_u64(mfb_arg(0), sp, 0x20)  // 5th arg bInheritHandles = TRUE` with no
`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, so every inheritable handle passes.

The temp-file path already uses `O_CLOEXEC` (bug-102) and the spawn self-pipe uses
`FD_CLOEXEC` — so the primitive exists; ordinary `fs::open` and sockets just don't
set it. `grep -rn 'CLOEXEC' src/codegen/builtins/fs src/codegen/os/socket` confirms
CLOEXEC appears only for temp files, not for `fs::open` or `socket()`.

## Reproduction

Agent-demonstrated: a parent opens a secret file, spawns `cat /dev/fd/N`, and the
child prints the secret. Lead code-verified the flag/handle sites above (Linux
flag words, bare `SOCK_STREAM`, Windows `bInheritHandles=TRUE`).

## Best fix

- Unix: OR `O_CLOEXEC` into every `fs::open` flag word and add `SOCK_CLOEXEC` to
  the `socket()`/`accept4()` type argument (fall back to a post-open
  `fcntl(FD_CLOEXEC)` where `accept4` is unavailable). Keep the fds that spawn
  *deliberately* passes (the child's stdio) as the only inherited ones — spawn
  already dups those explicitly.
- Windows: pass `bInheritHandles = FALSE` and hand the child its stdio via an
  explicit `STARTUPINFOEX` `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, so only the three
  intended handles cross.

## Non-goals

Do not break stdio redirection into the child (the pipe ends spawn dups are
intended); no MFBASIC surface change; keep the temp-file/self-pipe CLOEXEC as-is.

## Prior art

audit-2 recorded OS-01 (0o666 file mode, bug-184, fixed) — a different fs-helper
default. The fd-inheritance class is new here (searched `CLOEXEC`,
`bInheritHandles`, `SOCK_CLOEXEC`, `fd inherit` across `bugs/`, `bugs/completed/`,
`audit-1-*`, `audit-2-*`). bug-102 fixed CLOEXEC for temp fds only.
