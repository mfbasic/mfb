# bug-63: fs error paths leak a freshly-opened fd on record-alloc OOM, leave `mkstemps` temp files on disk on every atomic-write failure, and permit a double-close after a failed `close`

Last updated: 2026-07-09
Effort: small (<1h)

A cluster of LOW-severity fs resource-hygiene defects, all "an error path skips a cleanup
the success path performs". None corrupts memory; the impact is fd/temp-file leaks under
error conditions and a rare wrong-fd close.

**(1) fd leaked on record-alloc OOM.** After `open()`/`mkstemps` succeeds and returns a
valid fd, the subsequent `arena_alloc` of the File record can fail; the OOM branch reports
`ErrOutOfMemory` and jumps to the error tail **without `close(fd)`**, leaking the OS fd. A
loop opening under arena pressure exhausts fds (`EMFILE`). Sites:
`fs_helpers_io.rs:lower_fs_open_helper` (`:459-484`), `fs_helpers_atomic.rs:lower_fs_create_temp_file_helper`
(`:138-153`), `lower_fs_read_bytes_path_helper` (`:1426-1441`). Contrast: the normal error
tails after a successful open (write*Path/readTextPath) do close the fd first.

**(2) atomic-write temp file left on disk on every failure.** `lower_fs_atomic_write_helper`
creates a temp file via `mkstemps` (`<path>.mfb-XXXXXX.tmp`); on a write/fsync/close/
record-alloc/rename failure the error tails close the fd but never `unlink` the temp file,
so each failed atomic write litters the target directory with a stray temp file. Sites:
`fs_helpers_atomic.rs` `write_error`/`sync_error` (`:582-591`), `rename_error` (`:572-580`),
`alloc_error` (`:617`). Contrast: the success path renames temp→final, consuming the name.

**(3) double-close after a failed `close`.** `lower_fs_close_helper` (`:590-621`) sets
`FILE_OFFSET_CLOSED = 1` only on the success branch; a failed `close` (`EINTR`/`EIO`)
returns `ErrCloseFailed` with `CLOSED` still 0. On Linux the fd is already released, so a
subsequent `fs::close` — not seeing "already closed" — drains again and closes the same fd
number, which may now name a different open file → the wrong fd is closed. Contrast: the
`already_closed` guard (`:565-567`) works only once `CLOSED` is set, which the error path
never does.

The single correct behavior a fix produces: (1) every error exit after a successful open
closes the fd; (2) every atomic-write failure unlinks the temp file; (3) a failed `close`
still marks the File closed (the fd is consumed) while surfacing the error.

References: as cited above, all under `src/target/shared/code/fs_helpers_io.rs` and
`fs_helpers_atomic.rs`. KNOWN (not re-filed): OS-01 (0o666 perms), OS-03 (canonicalPath
TOCTOU), OS-04 (openFileNoFollow), OS-06 (socket fd leak). bug-44 covers the fsync/close
int-return width; bug-48 covers the `listDirectory` overflow. Found during the goal-01
review of `src/target/shared/code/`.

## Failing Reproduction

- (1) Open files in a loop under arena exhaustion → fd count climbs to `EMFILE` (visible
  in `/proc/<pid>/fd` or via `ulimit -n`).
- (2) `fs::atomicWrite` to a directory where the rename fails (e.g. cross-device target, or
  ENOSPC on write) in a loop → stray `*.tmp` files accumulate in the target dir.
- (3) `fs::close` a File whose `close` returns an error (fault-injected `EIO`), then
  `fs::close` it again → the second close acts on a since-reused fd number.

- Observed: leaked fds (1); temp-file litter (2); a second close of a reused fd (3).
- Expected: fd closed on OOM (1); temp unlinked on failure (2); `CLOSED` set even on close
  failure (3).

Contrast: the normal open-error tails close the fd; the atomic-write success path consumes
the temp; the `already_closed` guard works once `CLOSED` is set.

## Root Cause

Each error label branches to the error tail without the cleanup the success path performs:
(1) no `close(fd)` on the OOM branch; (2) no `unlink(temp)` on any atomic-write failure;
(3) `CLOSED` set only on the `close`-success branch.

## Goal

- (1) Every error exit after a successful `open`/`mkstemps` closes the fd.
- (2) Every atomic-write failure after `mkstemps` unlinks the temp file.
- (3) `lower_fs_close_helper` marks `CLOSED` regardless of the `close` result (surfacing
  the error but preventing a re-close).

### Non-goals (must NOT change)

- Success-path behavior.
- The bug-44 int-return normalization (separate) and OS-0x known findings.

## Blast Radius

- `lower_fs_open_helper`, `lower_fs_create_temp_file_helper`, `lower_fs_read_bytes_path_helper`
  OOM branches — item (1).
- `lower_fs_atomic_write_helper` failure tails — item (2).
- `lower_fs_close_helper` — item (3).

## Fix Design

(1) On the record-alloc OOM branch, restore `fd` to the syscall-arg register and
`emit_close_file` before the error tail. (2) Build the temp C-string early and `unlink` it
on every post-`mkstemps` failure. (3) Set `FILE_OFFSET_CLOSED` before/regardless of the
`close` result, then surface the error.

## Phases

### Phase 1 — tests

- [x] fd-count-under-OOM test; atomic-write-failure temp-litter test; failed-close
      double-close test. Codegen-structure invariants for (1)/(3) (deterministic OOM
      and close-fault injection are impractical on the dev host) plus a real runtime
      temp-litter test for (2). All confirmed failing on the pre-fix HEAD worktree.

### Phase 2 — the fixes

- [x] Add the close-on-OOM, unlink-on-failure, and mark-closed-on-error logic.

### Phase 3 — validation

- [x] `cargo test --test fs_error_path_hygiene` (5/5) and `--test fs_atomic_int_return`
      (4/4) green; happy-path fs program round-trips; runtime temp-litter reproduction
      shows litter on the pre-fix binary and none after. Goldens regenerated +
      `scripts/artifact-gate.sh` / `scripts/test-accept.sh` are the orchestrator's step.

## Validation Plan

- Regression test(s): the three fault-injected tests above.
- Runtime proof: fd count bounded under OOM; no temp litter after failed atomic writes; no
  wrong-fd close.
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

Three fs error-path hygiene gaps: an fd leaked on record-alloc OOM, a `mkstemps` temp left
on disk on atomic-write failure, and a double-close enabled by a failed `close` not marking
the File closed. All LOW, all "error path skips the success path's cleanup"; fixes are
local per site.

## Resolution

Fixed 2026-07-10.

**(1) fd leaked on record-alloc OOM.** In each of the three helpers, the File-record
`arena_alloc` OOM branch now restores the just-opened fd to `x0` and calls
`emit_close_file` before falling into the OOM error tail. `fd` is already a spilled vreg,
so it survives the failed `arena_alloc` and the close (compiler.md register lifetimes).
Sites: `fs_helpers_io.rs::lower_fs_open_helper` (covers `fs::openFile`/`openFileNoFollow`/
`open`); `fs_helpers_atomic.rs::lower_fs_create_temp_file_helper` and
`::lower_fs_read_bytes_path_helper`. createTempFile follows the doc scope (close only; the
temp file is the caller's product). Net effect in the emitted `ncode`: `openFile`/
`createTempFile` gain their first `close` call (0→1); `readBytes` goes 1→2.

**(2) atomic-write temp left on disk.** `lower_fs_atomic_write_helper` now `unlink`s the
temp file on every post-`mkstemps` failure via `emit_fs_path_operation(Unlink)`:
- the write/fsync/close convergence (`close_error`) unlinks before setting ErrOutput;
- the two post-`mkstemps` C-string alloc failures route to a new `unlink_alloc_error`
  label that unlinks then falls into `alloc_error`;
- rename failure routes to a new `rename_failed` label that captures the rename errno
  (`emit_errno` → `x9` → `saved_errno` vreg), unlinks (which itself sets errno), restores
  the saved errno, then joins the shared errno mapping at `rename_error_map`.
The pre-`mkstemps` temp-path alloc failure and the `mkstemps`-failure path (no temp on
disk) deliberately skip the unlink. This required adding the `unlink`/`_unlink` import to
the `fs.writeTextAtomic|writeBytesAtomic` arm of all four per-target `plan.rs` files, since
`emit_fs_path_operation` resolves its libc symbol from the program's import table at emit
time. Also fixed a latent adjacent defect surfaced by the change: `emit_errno_error_mapping`'s
generic `err_output` case does not branch to `done`, so the errno tail previously fell
through into the write/sync close tail and re-closed the fd (a garbage fd vreg on
`mkstemps` failure; an already-closed fd on rename failure) — an explicit `branch(done)`
now terminates the errno path.

**(3) double-close after a failed `close`.** `lower_fs_close_helper` now stores
`FILE_OFFSET_CLOSED = 1` **before** branching on the `close` result, so a failed close
(EINTR/EIO, where Linux has still released the fd) leaves the File marked closed; the
`already_closed` guard then refuses a re-close while `ErrCloseFailed` still surfaces once.

Files changed: `src/target/shared/code/fs_helpers_io.rs`,
`src/target/shared/code/fs_helpers_atomic.rs`, `src/target/{macos_aarch64,linux_aarch64,
linux_x86_64,linux_riscv64}/plan.rs` (unlink import), and `tests/fs_error_path_hygiene.rs`
(new: 4 codegen-structure tests across all backends + 1 host runtime temp-litter test).

Validation: `cargo test --test fs_error_path_hygiene` 5/5, `--test fs_atomic_int_return`
4/4. Runtime proof for (2): building the same atomic-write-onto-a-directory program with a
pre-fix HEAD worktree left `collide.mfb-XXXXXX.tmp` behind; the fixed compiler leaves the
directory clean. Structural signals verified against the pre-fix worktree (openFile/
createTempFile 0 closes→≥1, readBytes 1→2, atomic-write 0 unlinks→3, and the CLOSED store
moving before the close-result branch). Goldens for fs-helper `ncode`/`mir`/`mfp` (and the
new atomic-write `unlink` import) will shift; regeneration + the full acceptance suite are
the orchestrator's step.
