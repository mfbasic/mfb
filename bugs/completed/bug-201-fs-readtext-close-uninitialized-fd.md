# bug-201: fs::readText pre-open alloc failure closes an uninitialized fd vreg (may close a live fd)

Last updated: 2026-07-14
Effort: small (<1h)
Severity: MEDIUM
Class: memory-safety

Status: Fixed (2026-07-15) — `lower_fs_read_text_path_helper`'s `alloc_error` tail
is now close-free (reached from the pre-open C-string alloc failure, before `fd`
is assigned); the post-open result-String alloc failure closes `fd` inline before
branching to it. Mirrors `lower_fs_open_helper`, so only the post-open path closes
the fd.
Regression Test: normal `fs::readText` verified at runtime (reads a file
correctly); the pre-open OOM path no longer closes an uninitialized fd vreg. The
OOM path itself is not directly runtime-triggerable (arena alloc failure).

`lower_fs_read_text_path_helper` shares an `alloc_error` tail that
unconditionally `close()`s `fd`. But the **pre-open** C-string allocation failure
branches to that tail *before* `fd` is ever assigned, so `close()` runs on an
uninitialized vreg — whatever the register/slot happened to hold, possibly a live
fd (stdout/stderr or another open file). This is a regression from the bug-101
fix, which added the fd close to the string-alloc path but routed the pre-open
failure through the same label. Sibling helpers `lower_fs_open_helper` and
`lower_fs_read_bytes_path_helper` correctly use a close-free OOM label for the
pre-open case.

## Failing Reproduction

`fs::readText(path)` when the first `_mfb_arena_alloc(len0+1)` (the C-string
buffer, before `open`) returns `ErrOutOfMemory`. Observed: control goes
entry→alloc→fail→`branch(&alloc_error)` (`:1130`); `alloc_error`
`move_register(return_register(), &fd)` + `emit_close_file` closes an
uninitialized `fd` (only written at `open_ok`, `:1167`). Expected: report
`ERR_OUT_OF_MEMORY` without closing any fd.

## Root Cause

`src/target/shared/code/fs_helpers_atomic.rs:1330-1338` (`alloc_error` tail)
reached from `:1130` (pre-open failure) before `fd` is assigned.

## Non-goals

- Do not change the post-open string-alloc failure path (it *should* close fd).
- Do not alter the correct sibling helpers.

## Blast Radius

- `lower_fs_read_text_path_helper` only.

## Fix Design

Give the pre-open C-string alloc failure its own OOM exit (report
`ERR_OUT_OF_MEMORY` without closing), mirroring `lower_fs_open_helper`
(`io.rs:604-613`); only the post-open string-alloc failure should reach the
fd-closing tail.
