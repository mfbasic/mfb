# bug-701: `fs` classifies the same host refusal differently by member and by platform

Last updated: 2026-09-24
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: extend the bug-694 probe matrix into `tests/runtime/` per sub-issue (Phase 1)

bug-694's cross-platform probe (every `fs` member against the same fixtures on
macOS, Linux aarch64 and Win11) showed that which error a program must `TRAP`
for depends on which member it called and which host it runs on, for the same
condition. One case is plainly wrong — reading a directory on Linux reports
`ErrOutOfMemory` — and the rest are inconsistencies a program cannot reasonably
code against.

**Correct behavior.** One condition, one error: the same refusal raises the same
`errorCode` whichever `fs` member met it and whichever host it ran on, and no
read path reports a write failure.

References:

- bug-694 (`bugs/completed/bug-694-fs-man-pages-declare-no-errors.md`): the
  probe, the Errors tables, and the fixes that were clear-cut (C1, C3, C5–C7).
  Its pages document today's per-platform behavior; they change with this bug.
- `mfb spec diagnostics error-codes` — the error-code meanings.

## Failing Reproduction

The bug-694 probe: one `SUB` per case, each `TRAP`ping one `fs` call and
printing `e.code`, run against fresh fixtures (`fixtures.sh` / `fixtures.ps1`
in the bug-694 run; rebuild them in Phase 1 as a committed harness). Minimal
cases:

```
IMPORT fs
IMPORT io

SUB main()
  LET a = fs::readText("/tmp") TRAP(e)
    io::print("readText dir " & toString(e.code))
    RECOVER ""
  END TRAP
  fs::writeText("/tmp/no/such/dir/x", "x") TRAP(e)
    io::print("writeText missing dir " & toString(e.code))
    RECOVER
  END TRAP
  RES f = fs::openFile("/tmp/no/such/dir/x", "w") TRAP(e)
    io::print("openFile missing dir " & toString(e.code))
    EXIT SUB
  END TRAP
END SUB
```

Measured (`e.code` decoded):

| case | macOS | Linux aarch64 | Windows |
| --- | --- | --- | --- |
| `readText` / `readBytes` of a directory | ErrReadFailed | **ErrOutOfMemory** | ErrAccessDenied |
| `writeText` into a missing directory | ErrNotFound | ErrNotFound | ErrNotFound |
| `openFile(…, "w")` into a missing directory | ErrPathNotFound | ErrPathNotFound | ErrPathNotFound |
| `readText` a too-long name | ErrInvalidPath | ErrInvalidPath | ErrPathNotFound |
| `writeText` a too-long name | ErrWriteFailed | ErrWriteFailed | ErrNotFound |
| `readText` through a file used as a directory | ErrInvalidPath | ErrInvalidPath | ErrPathNotFound |
| `writeText` through a file used as a directory | ErrWriteFailed | ErrWriteFailed | ErrNotFound |
| `writeText` / `openFile(…,"w")` / `deleteFile` of a directory | ErrWriteFailed | ErrWriteFailed | ErrAccessDenied |
| `deleteDirectory` / `setCurrentDirectory` of a file | ErrInvalidPath | ErrInvalidPath | ErrWriteFailed |
| `canonicalPath` through a file used as a directory, or a link loop | ErrWriteFailed | ErrWriteFailed | ErrNotFound |
| `createDirectories` through a file | ErrWriteFailed | ErrWriteFailed | ErrNotFound |

## Root Cause

Four separate mechanisms:

1. **Linux directory read (C2 in bug-694).** `readText`/`readBytes` measure the
   file with `lseek(SEEK_END)` before reading
   (`gen_atomic_write.rs:lower_fs_read_text_path_helper`,
   `lower_fs_read_bytes_path_helper`). Hypothesis, to confirm in Phase 1: on an
   ext4 directory fd `SEEK_END` returns the htree end-of-directory offset (up to
   `i64::MAX`), so the result buffer request for that "size" fails and the
   member raises `ErrOutOfMemory`. macOS reports a small size, and the following
   `read()` fails `EISDIR` (`ErrReadFailed`). Windows refuses the open itself.
   `readAll`/`readAllBytes`/`size` on an `openFile(dir, "r")` handle use the same
   measurement and are expected to behave the same (unmeasured).
2. **Two errno mappers.** `emit_errno_error_mapping` (`gen_shared.rs`; used by
   the write/append/atomic writers, `createTempFile`, `listDirectory`,
   `canonicalPath`, `isWithin`) maps a missing entry to `ErrNotFound` and has no
   `ErrInvalidPath` arm, while `emit_fs_path_errno_error_mapping` (open, read by
   path, delete, create, chdir) maps it to `ErrPathNotFound` and routes
   ENOTDIR/ENAMETOOLONG/EILSEQ/ELOOP to `ErrInvalidPath`.
3. **One fallback for every direction.** Both mappers send any unlisted host code
   to `ErrWriteFailed` ("Write or flush operation failed."), including a failed
   open for reading (`EMFILE`, `EIO`, …) and a failed `chdir`.
4. **Windows codes the mappers do not list.** `ERROR_DIRECTORY` (267, "not a
   directory") and the codes Windows gives for a too-long or file-as-directory
   path fall to the fallback or to not-found, so the Windows column diverges from
   the POSIX one.

A fifth, documentation-visible consequence: both mappers emit every arm for
every member, so members whose host call can never produce `EEXIST`/`ENOTEMPTY`
(a read-only open, `chdir`, `opendir`, `realpath`) still carry
`ErrAlreadyExists`/`ErrResourceBusy` in their Errors tables. bug-694's pages say
so explicitly ("reading a file never produces either").

## Goal

- Each row of the table above reads the same across members that meet the same
  condition, and across the three hosts.
- `readText`/`readBytes` of a directory raise one documented error everywhere,
  never `ErrOutOfMemory`.
- A failed open for reading never raises `ErrWriteFailed`.
- Each member's Errors table carries only codes its host call can produce; the
  bug-694 guard (`every_fs_member_declares_exactly_the_errors_its_lowering_raises`)
  then forces the descriptors down to match.

### Non-goals (must NOT change)

- The error-code table itself (no renumbering, no new constants without a
  separate decision).
- Behavior that is already consistent (the rows not in the table above).

## Blast Radius

- Every `fs` member that reaches either mapper (all path-taking members).
- The bug-694 `fs` man pages: each page's errors paragraph states today's
  per-platform behavior and must be rewritten with the fix.
- `createDirectories`' own mapper (`gen_directory.rs:lower_fs_create_directories_helper`).

## Fix Design

Recommended: one mapper, parameterized by the operation (read / write / create /
remove / resolve) and the platform family, emitting only the arms that operation
can reach. Missing entry → `ErrPathNotFound` everywhere; unusable path (too long,
file used as a directory, link loop, `ERROR_DIRECTORY`) → `ErrInvalidPath`
everywhere; fallback → `ErrReadFailed` for read operations and `ErrWriteFailed`
for write operations. A directory where a file is required is refused at open
with one code on every host.

## Phases

### Phase 1: failing tests + measurement (no behavior change)

- [ ] Commit the probe as a runtime test per row (macOS/Linux in `cargo test`;
      the Windows column as a recorded box-2230 run).
- [ ] Confirm or refute the C2 hypothesis (`strace` the Linux read).

Commit: —

### Phase 2: the fix

- [ ] Replace the two mappers with the operation-aware one; fix the directory
      read; update the Errors tables (the guard forces this) and the pages.

Commit: —

### Phase 3: validation

- [ ] Full `cargo test`; artifact gate; the probe on all three hosts.

Commit: —

## Validation Plan

- Regression tests: the per-row runtime tests.
- Runtime proof: the probe matrix, identical across hosts.
- Doc sync: every `fs` man page errors paragraph.
- Full suite: `cargo test`, `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions

- Missing entry: `ErrPathNotFound` for every path-taking member (recommended —
  it is the filesystem-specific code) vs. keeping `ErrNotFound` on the writers.
  This changes what existing `TRAP` handlers see on the writers.
- Directory where a file is required: which code — `ErrInvalidPath`
  (recommended; the path cannot name a file) or `ErrAccessDenied` (Windows'
  current answer).

## Summary

The Linux directory read is a plain bug; the rest are public-surface choices
about which code a condition raises, which is why they are not folded into
bug-694. The risk is in changing codes existing programs trap on.
