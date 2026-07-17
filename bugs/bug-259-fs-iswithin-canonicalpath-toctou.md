# bug-259: `isWithin`/`canonicalPath` containment is a check-then-open TOCTOU (no `openat2`/`RESOLVE_BENEATH`)

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: (none yet)

The path-containment guard MFBASIC programs use to keep an
attacker-controlled filename inside an intended directory is a `realpath`-based
**check** that is separate from the later `open`. Between the containment check
and the open, an attacker who can rename/relink a path component swaps a
validated path for one that escapes the sandbox — a classic time-of-check /
time-of-use race. The single correct behavior a fix produces: containment is
enforced atomically with the open (kernel-resolved), so a component swap after
the check cannot redirect the open outside the intended root.

References:

- `planning/audit-2-fs-net-thread.md` (OS-03).
- `planning/old-plans/audit-1-*` (original OS-03).
- `src/target/shared/code/fs_helpers_paths.rs:1410` — `isWithin` is a
  `realpath`-based containment check; there is no `openat2(RESOLVE_BENEATH)` /
  `O_NOFOLLOW`-anchored open bound to the checked directory fd.

## Failing Reproduction

A program that validates `isWithin(root, userPath)` and then opens `userPath`.
Between the two calls, an attacker (sharing the directory, or controlling a
subdirectory) replaces an intermediate component with a symlink pointing outside
`root`. Observed: the open follows the swapped component and reads/writes outside
`root` even though `isWithin` returned true. Expected: the open resolves within
`root` or fails.

Contrast: `createTempFile`/`atomicWrite` do not expose a caller-supplied
multi-component path in the same check-then-open shape.

## Root Cause

`isWithin` canonicalises and string-compares against `root` at check time
(`fs_helpers_paths.rs:1410`); the subsequent `open` re-resolves the path from
scratch through the (now-mutated) directory tree. The two resolutions are not the
same syscall and are not anchored to a pinned directory fd, so any component
along the path can change in between.

## Goal

- Path containment is enforced by the kernel at open time (Linux `openat2` with
  `RESOLVE_BENEATH`/`RESOLVE_NO_SYMLINKS` anchored to an O_PATH fd of `root`; a
  best-effort `O_NOFOLLOW`/component-walk fallback where `openat2` is
  unavailable), so a post-check component swap cannot escape `root`.

### Non-goals (must NOT change)

- The public `isWithin`/`canonicalPath` surface or their return semantics for
  the non-adversarial case.
- Turning every `fs::open` into a sandboxed open — only the containment-guarded
  paths.

## Fix Design

Provide an internal "open-beneath" primitive: open `root` `O_PATH|O_DIRECTORY`,
then `openat2` the relative remainder with `RESOLVE_BENEATH`. Route the
containment-guarded open through it so the check and the open share one
kernel-atomic resolution. On kernels without `openat2`, fall back to a
component-by-component `openat(O_NOFOLLOW)` walk from the pinned fd (this also
subsumes bug-260/OS-04). macOS has no `openat2`; use the `O_NOFOLLOW` walk there.
