# bug-260: `openFileNoFollow` guards only the final path component; intermediate directory symlinks are still followed

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security

Status: Fixed
Regression Test: `tests/rt-behavior/fs/fs-nofollow-symlink-rt` — extended with an
intermediate directory symlink (`data/linkdir -> realdir`): opening
`linkdir/secret.txt` now traps `ErrAccessDenied` (77030003), while a clean
`realdir/secret.txt` still opens. Hardware-validated: x86_64/aarch64/riscv64 ×
glibc/musl (a repro with an intermediate + a final symlink; both refused, a
non-symlink path opens) and macOS.

## Resolution

`openFileNoFollow` now refuses a symlink at ANY path component, not just the last:

- **macOS**: the no-follow flag set (`open_flag_set`) uses `O_NOFOLLOW_ANY`
  (`0x2000_0000`) instead of `O_NOFOLLOW` (`0x100`). O_NOFOLLOW_ANY fails with
  ELOOP if a symlink is met at any component, in one `open()` with no walk.
- **Linux**: `lower_fs_open_helper` routes the Linux no-follow case through
  `openat2(AT_FDCWD, path, &open_how{flags, mode, resolve=RESOLVE_NO_SYMLINKS}, 24)`
  via the libc `syscall` wrapper (import added to the three Linux plans). The
  `open_how.mode` is `0o600` only when `O_CREAT` is set (openat2 rejects a nonzero
  mode otherwise with EINVAL) and `0` for read/read-write. On `ENOSYS` (a kernel
  older than 5.6, or a restrictive seccomp filter) it falls back to the prior
  plain `open` + terminal `O_NOFOLLOW` (best-effort). The syscall number is passed
  in `ARG[0]`, not the return register — a def in `%ret0` (call-clobbered) with no
  use before the call is dropped on aarch64 (found and fixed during remote
  validation: aarch64/riscv64 initially failed, x86_64 passed).

ELOOP maps to `ErrAccessDenied` for a no-follow open, as before. The plain
follow-allowed `fs::open`/`fs::openFile` path is untouched, and the 24-byte
`open_how` stack local is reserved only for the Linux no-follow flavor (every
other flavor keeps a byte-identical frame). Man page updated to state the
whole-path guarantee.

Composes with bug-259 (OS-03): the same kernel-anchored resolution primitives
(openat2 RESOLVE_*, O_NOFOLLOW_ANY) are the building blocks for the containment
open there.

`fs::openFileNoFollow` is meant to refuse to open a symlink, but it adds
`O_NOFOLLOW` to the **terminal** `open` only. Every intermediate directory in the
path is still resolved with symlink-following, so an attacker who controls (or
can plant a symlink at) any parent component redirects the open to a different
directory subtree — defeating the guarantee the API name promises. The single
correct behavior a fix produces: `openFileNoFollow` refuses to traverse a symlink
at **any** component of the path, not just the last.

References:

- `planning/audit-2-fs-net-thread.md` (OS-04).
- `planning/old-plans/audit-1-*` (original OS-04).
- `src/target/shared/code/fs_helpers_io.rs:2191` — `O_NOFOLLOW` is OR'd into the
  final `open` flags only; no per-component `O_NOFOLLOW` walk.

## Failing Reproduction

`fs::openFileNoFollow("/base/link/secret")` where `/base/link` is a symlink to
`/etc`. Observed: the call opens `/etc/secret` — the intermediate symlink `link`
was followed; only a symlink *at* `secret` would have been refused. Expected: the
open fails because a symlink was encountered while resolving the path.

Contrast: a direct symlink at the final component (`/base/secret -> /etc/passwd`)
is correctly refused today — that is the only case the flag covers.

## Root Cause

`O_NOFOLLOW` constrains only the last component of the path passed to `open`
(`fs_helpers_io.rs:2191`). The kernel still follows symlinks for all preceding
directory components during path resolution, so the no-follow guarantee is
single-component, not whole-path.

## Goal

- `openFileNoFollow` rejects a symlink encountered at any component: resolve the
  path component-by-component with `openat(dirfd, comp, O_NOFOLLOW)` from a pinned
  root fd (or `openat2` with `RESOLVE_NO_SYMLINKS` where available), so no
  intermediate symlink is traversed.

### Non-goals (must NOT change)

- The `openFileNoFollow` public surface / error contract for the non-symlink
  case.
- The behavior of the ordinary (follow-allowed) `fs::open`.

## Fix Design

Replace the single terminal-`O_NOFOLLOW` open with a component walk: split the
path, `openat(O_PATH|O_NOFOLLOW|O_DIRECTORY)` each intermediate directory from the
previous fd, then `openat(O_NOFOLLOW)` the final component. Share the
open-beneath primitive proposed in bug-259/OS-03 (the two fixes compose — a
pinned-fd component walk closes both the intermediate-symlink and the TOCTOU
gaps). On kernels with `openat2`, `RESOLVE_NO_SYMLINKS` does this in one syscall.
