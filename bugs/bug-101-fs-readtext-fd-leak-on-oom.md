# bug-101 — `fs::readText(path)` leaks the open fd when the result-string allocation OOMs

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G10).
**Severity:** MED — fd leak on a catchable error; repeated calls exhaust the
process fd table.
**Class:** memory-safety (fd leak on error path).

## Finding

`src/target/shared/code/fs_helpers_atomic.rs:1108-1111` and :1215
(`lower_fs_read_text_path_helper`). The helper opens its own fd
(`emit_open_file`), seeks end/start to size the file, then calls
`_mfb_arena_alloc` for the result String. On allocation failure it does
`branch(&alloc_error)`, and the `alloc_error` tail (line 1215) only sets
`ERR_OUT_OF_MEMORY` and returns — it **never closes `fd`**.

Every other internally-opened path helper closes the fd on its post-open
failure paths: `lower_fs_read_bytes_path_helper`
(fs_helpers_atomic.rs:1560-1573) and `lower_fs_open_helper`
(fs_helpers_io.rs:725-738) both close before OOM (the bug-63 fix), and this
helper's own seek-failure path routes through `close_and_read_error` which
closes. The string-alloc OOM path is the sole post-open exit that skips the
close.

## Trigger

`fs::readText("bigfile")` where the file length is large enough (near the
arena/address-space ceiling) that `arena_alloc(length+9)` returns
`ErrOutOfMemory`. The program catches the error and continues; the OS fd is
leaked. Repeated calls exhaust the fd table.

## Fix

Close `fd` on the `alloc_error` path before setting `ERR_OUT_OF_MEMORY` —
route it through `close_and_read_error` like the seek-failure path, or emit an
explicit close.

## Prior art

bug-63 covered File-record alloc leaks in open/readBytes/createTempFile, not
this String alloc in readText.
