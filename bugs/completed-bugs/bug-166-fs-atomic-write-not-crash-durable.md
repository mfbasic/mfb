# bug-166 — `fs::writeTextAtomic`/`writeBytesAtomic` is not crash-durable: the parent directory is never fsynced after rename

Last updated: 2026-07-12
Severity: MEDIUM — a successful "atomic" write can vanish or revert across a crash/power loss.
Class: Correctness (durability).
Status: FIXED
Resolution: after a successful rename, `lower_fs_atomic_write_helper` derives the
parent directory from the final-path C-string (last-slash scan; "." / "/"
special cases), opens it `O_RDONLY`, `fsync`s, and closes it before reporting Ok.
Directory fsync is best-effort (the atomic rename already succeeded, so a
non-fsyncable directory must not fail the write). `tests/fs_atomic_int_return.rs`
updated (the new best-effort dir fsync carries no sign-extend seam, like the
cleanup closes); durability contract documented in the fs man pages.

## Finding

`src/target/shared/code/fs_helpers_atomic.rs` — the atomic-write helper fsyncs
the temp file's data (`emit_sync_file` on `fd`, :530-535), closes, then `rename`s
temp→final (:609-614). It never opens+fsyncs the containing **directory**, so the
rename's directory-entry update may not be persisted. After a crash the target
can be absent, or still point at the old inode — the write is atomic w.r.t.
concurrent readers but not durable across a crash, which is the stronger
guarantee an `...Atomic` API implies.

## Trigger

`fs::writeTextAtomic(path, ...)` / `writeBytesAtomic` returns Ok; the machine
loses power / crashes before the containing directory's metadata is flushed to
disk. On next boot the file is missing or holds the pre-write contents despite
the Ok return.

## Fix

After a successful rename, `open(dir, O_RDONLY)` + `fsync` + `close` the parent
directory (derived from the path) before returning Ok. Document the durability
contract in the `fs` man page. (Note: this is a design gap, not a regression;
rank MEDIUM because the API name implies durability that isn't delivered.)
