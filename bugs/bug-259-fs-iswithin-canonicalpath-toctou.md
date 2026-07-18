# bug-259: `isWithin`/`canonicalPath` containment is a check-then-open TOCTOU (no `openat2`/`RESOLVE_BENEATH`)

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security

Status: Assessed — requires a NEW public open-beneath API (a feature, not an
isolated code fix). Concrete design below; the kernel primitives it needs are now
in place (landed by bug-260). Not implemented in isolation because it adds public
surface whose shape/semantics is a design decision.
Regression Test: (pending the feature)

## Assessment

The finding is a check-then-open TOCTOU: at the language level a program calls
`fs::isWithin(root, path)` and *then* `fs::open(path)` — two separate operations,
so no change to `isWithin`/`canonicalPath` (both pure `realpath` checks, verified
at `fs_helpers_paths.rs`) can make them atomic. Closing the race requires the
kernel to enforce containment **at open time**, which means a new primitive the
program calls *instead of* isWithin+open — i.e. a new public API such as
`fs::openBeneath(root AS String, relPath AS String[, mode AS String]) AS File`.

### Concrete design (ready to build)

Codegen helper `lower_fs_open_beneath`:
1. `open(root, O_DIRECTORY | O_RDONLY | O_CLOEXEC)` → `rootfd` (following symlinks
   in `root` is fine — `root` is the trusted base).
2. Reject a `relPath` that is absolute or contains a `..` component (a cheap
   runtime scan) so it cannot escape upward.
3. **Linux**: `openat2(rootfd, relPath, &open_how{flags, mode,
   resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS}, 24)` via the libc `syscall`
   wrapper — the kernel refuses any escape (`..`, absolute, or a symlink pointing
   outside `rootfd`) atomically. `ENOSYS` → fall back to `openat(rootfd, relPath,
   O_NOFOLLOW)` (best-effort, matching the bug-260 fallback).
4. **macOS**: `openat(rootfd, relPath, flags | O_NOFOLLOW_ANY)` — anchored at
   `rootfd`, no symlink traversal, and with the step-2 `..` rejection the result
   is guaranteed beneath `root`.
5. `close(rootfd)`; build the `File` record from the resulting fd.

The kernel primitives are already wired: `openat2`/`open_how`/`RESOLVE_*` and
`O_NOFOLLOW_ANY` all landed with **bug-260** (`fs_helpers_io.rs`,
`open_flag_set`, the three Linux plans' `syscall` import). The remaining work is
(a) `openat` as a platform emission (only `open` exists today), (b) the new
builtin surface across `src/builtins/fs.rs` (~10 registration points: arg names,
arity, return/param types, dispatch), the `mod.rs` codegen dispatch, and the four
target plans, and (c) man page + `mfb spec` + valid/invalid func tests for both
overloads.

### Why not landed here

This is a new public API — its name, parameter shape, `..`/absolute-path policy,
and error contract are a surface decision that belongs in a `plan-NN` feature
doc, not an isolated bug patch (per "production-ready only": a rushed, unratified
public API is worse than a scoped plan). Recommend authoring `plan-NN
fs::openBeneath` and building it on the bug-260 primitives; it also subsumes the
containment half of OS-03 once the atomic open replaces the isWithin+open pattern
in caller code.

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
