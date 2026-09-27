# bug-701: `fs` classifies the same host refusal differently by member and by platform

## STATUS: FIXED (23e21c239)

Implemented the recommended design and both adopted defaults (`ErrPathNotFound`
for every missing entry, `ErrInvalidPath` for a directory where a file is
required). Deviations: the mapper also maps POSIX `EPERM` to `ErrAccessDenied`
(it fell to `ErrWriteFailed`), `EISDIR` and Windows `ERROR_DIRECTORY` /
`ERROR_CANT_RESOLVE_FILENAME` to `ErrInvalidPath`, and `rmdir`'s `EEXIST` to
`ErrResourceBusy` — the same one-condition-one-code rule applied to codes the
table did not list; `openFile(dir, "r")` is refused on POSIX too (Windows
already refused it), so no handle to a directory exists to read through. The
Windows link-loop row is unmeasured (no symlink privilege on box 2230) and the
Linux x86_64 column is unmeasured (boxes 2227/2228 down).

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Fixed (see Phases) — **decided 2026-09-25 (owner): every target raises the same error for the
same condition. No per-platform error differences, and no man page documents one.**
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

- **One condition, one error, on every target** (owner decision, 2026-09-25). Each
  row of the table above reads the same across members that meet the same condition,
  and across macOS, Linux and Windows. A per-platform difference is a bug, not a
  documented behavior.
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
  per-platform behavior and must be rewritten with the fix. After it, no `fs` page
  names a platform in its errors paragraph.
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

- [x] Commit the probe as a runtime test per row (macOS/Linux in `cargo test`;
      the Windows column as a recorded box-2230 run). Each test asserts the one code
      below, so the same expectation runs on every host.
      `tests/runtime/rt_fs_host_error_classification.rs` — one program, one row per
      line of the target table (17 on Unix; the link-loop row needs a symlink, so it
      is Unix-only), fixtures built with `std::fs`. RED at 0cd2db799
      (`MFB_TEST_EXE=<base>/target/debug/mfb cargo test --test
      rt_fs_host_error_classification`: 12 of 17 rows wrong, e.g. `readText dir:
      77020001`, `writeText missing dir: 77050004`, `openFile r dir: ok`), GREEN
      after. The full bug-694 probe is committed as `tools/fs-error-matrix/`.
- [x] Confirm or refute the C2 hypothesis (`strace` the Linux read).
      **Confirmed** on box 2226 (Debian 12 aarch64, ext4), base build:
      `openat("/tmp/l701c2", O_RDONLY) = 3`, `lseek(3, 0, SEEK_END) =
      9223372036854775807`, then `mmap(NULL, 9223372036854779904, …) = -1 ENOMEM`,
      and the program printed `readText dir 77010001`.
- [x] Raw Windows codes, measured on box 2230 with a PowerShell P/Invoke of the
      same Win32 calls (2026-09-26): a read or write open of a directory,
      `DeleteFileW` of a directory and `MoveFileExW` onto one → 5; a missing parent,
      a file used as a directory (at any depth) and an over-long name → 3 (as does
      `GetFileAttributesW` for all three); a missing final entry → 2;
      `RemoveDirectoryW`/`SetCurrentDirectoryW`/`FindFirstFileW` given a file → 267;
      a non-empty `rmdir` → 145; `CreateDirectoryW` over a file → 183. So Windows
      cannot tell a missing parent from a file used as a directory by code alone.

Commit: see Phase 2 (test and fix land together)

### Phase 2: the fix

- [x] Replace the two mappers with the operation-aware one; fix the directory
      read; update the Errors tables (the guard forces this) and the pages.
      `gen_shared::emit_fs_host_error_mapping(op: FsHostOp, path, fallback)`
      replaces `emit_errno_error_mapping` and `emit_fs_path_errno_error_mapping`
      at all twelve sites, and `createDirectories`' private mapper. Where the host
      code names two conditions (Windows 3 and 5, POSIX `EPERM` from `unlink`) it
      asks a new shared helper, `_mfb_rt_fs_path_class`
      (`gen_classify.rs`), on the failure path only: *directory*, *unusable*
      (a component over 255 — UTF-16 units on Windows — or the nearest existing
      ancestor is not a directory), or *other*. The directory read: a POSIX
      read-only open of a directory succeeds, so the `open` family `stat`s after a
      successful read-only open and refuses a directory, and `readText`/`readBytes`
      classify on their read-failure paths (no cost on the success path). New
      arms beyond the table, same rule: POSIX `EPERM` → `ErrAccessDenied` (was the
      `ErrWriteFailed` fallback), `EISDIR` → `ErrInvalidPath`, Windows 267 and 1921
      → `ErrInvalidPath`, `rmdir` `EEXIST` → `ErrResourceBusy`. The declared lists
      shrank to what each call can reach (no `ErrAlreadyExists`/`ErrResourceBusy`
      on a read, open or `unlink`), and the "the table also lists …" sentences went
      with them; no `fs` page names a platform in its errors paragraph. Spec: new
      topic `mfb spec stdlib fs-host-errors`.

Commit: 23e21c239; 41c9ed2f1 (the new helper classified in
`codegen_helper_scratch_release`)

### Phase 3: validation

- [x] Full `cargo test`; artifact gate; the probe on all three hosts.
      Probe (`tools/fs-error-matrix`): macOS (local) and Linux aarch64 glibc (box
      2226) identical on every row; Windows (box 2230) identical on every row of the
      target table — its remaining differences are fixtures Windows cannot make
      (symlink loops without privilege; the `ro`/`nox` ACL fixtures). Linux x86_64
      boxes 2227/2228 were down (ssh refused) — that column is unmeasured; its
      errno numbers are Linux's, the same as aarch64's.
      `scripts/artifact-gate.sh target/release/mfb all` at 23e21c239: 40 diffs in 8
      fixtures, all reaching a changed lowering (regenerated in 3ea85fa20). Full
      `cargo test --no-fail-fast` at 3ea85fa20: 243 binaries ok, one red
      (`every_fs_helper_releases_the_path_it_marshalled` — the new
      `_mfb_rt_fs_path_class` helper was unclassified), fixed in 41c9ed2f1 and
      green re-run alone.

Commit: 3ea85fa20 (goldens)

## Validation Plan

- Regression tests: the per-row runtime tests.
- Runtime proof: the probe matrix, identical across hosts.
- Doc sync: every `fs` man page errors paragraph.
- Full suite: `cargo test`, `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions

- ~~Per-platform differences acceptable?~~ **Decided 2026-09-25 (owner): no. All
  targets raise the same error.**
- Missing entry: **`ErrPathNotFound` for every path-taking member** (the
  filesystem-specific code), replacing `ErrNotFound` on the writers. This changes
  what existing `TRAP` handlers see on the writers. Adopted as the default on
  2026-09-25 — it was presented to the owner with the decision above and not
  objected to.
- Directory where a file is required: **`ErrInvalidPath`** everywhere (the path
  cannot name a file), replacing Windows' `ErrAccessDenied`. Adopted as the default
  on the same terms.

Target table after the fix (every host):

| case | error |
| --- | --- |
| `readText` / `readBytes` of a directory | ErrInvalidPath |
| `writeText` / `openFile(…, "w")` into a missing directory | ErrPathNotFound |
| `readText` / `writeText` a too-long name | ErrInvalidPath |
| `readText` / `writeText` through a file used as a directory | ErrInvalidPath |
| `writeText` / `openFile(…,"w")` / `deleteFile` of a directory | ErrInvalidPath |
| `deleteDirectory` / `setCurrentDirectory` of a file | ErrInvalidPath |
| `canonicalPath` through a file used as a directory, or a link loop | ErrInvalidPath |
| `createDirectories` through a file | ErrInvalidPath |

## Summary

The Linux directory read is a plain bug; the rest are public-surface choices
about which code a condition raises, which is why they are not folded into
bug-694. The risk is in changing codes existing programs trap on.
