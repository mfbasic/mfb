# bug-260: `openFileNoFollow` guards only the final path component; intermediate directory symlinks are still followed

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: (none yet)

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
