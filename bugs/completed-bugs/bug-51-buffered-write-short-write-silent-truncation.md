# bug-51: single-`write()` output paths treat any non-negative return as complete — a short write silently drops data and reports success (both `fs::writeAll` buffered shortcuts and the default `io::print`/`io::write` stdout path)

Last updated: 2026-07-09
Effort: small (<1h)

Multiple output paths emit **one** `write(fd, src, len)` and then `compare 0 /
branch_lt write_error / branch appended` — treating any non-negative return as a
complete write. `write()` may return a short count (`0 < n < len`) on a filling disk, a
pipe/FIFO, or a signal-interrupted large transfer; these paths take the positive short
return as full success, never write the remaining bytes, and return `RESULT_OK`. The
result is silent data loss with no error. Two subsystems share the defect and the fix:

1. **`fs::writeAll` buffered shortcuts** (`emit_append_to_file_buffer`): the
   alloc-failed fallback and the "chunk larger than the 4096-byte buffer" path.
   Reached via `fs::setBuffered(file, TRUE)` then a large write.
2. **The default `io::print`/`io::write`/`writeLine` stdout path**
   (`lower_io_write_helper`): the direct write, the newline write, the oversized-chunk
   direct write, the alloc-failed fallback, and the prompt write in
   `lower_io_read_line_helper`. Buffering is **off by default**, so this is the common
   path — a large `io::print` to a full-disk redirect drops its tail and reports OK.

The single correct behavior a fix produces: every output path writes **all** the bytes
(looping on short counts) or reports `ErrOutput` — never claims success after a partial
write. The buffered stdout drain (`lower_stdout_drain`) already does this and is the
template.

References:

- `src/target/shared/code/fs_helpers_io.rs:emit_append_to_file_buffer`, alloc-failed
  path `:110-120`, large-chunk path `:139-149` — single `write` + `branch_lt`.
- `src/target/shared/code/io_helpers.rs:lower_io_write_helper` direct path `:259-279`,
  alloc-failed fallback `:113-122`, oversized-chunk write `:143-150`, newline write
  `:280-302`; `lower_io_read_line_helper` prompt write `:1349-1364`.
- The one-shot primitive: `emit_write` (per backend, e.g.
  `src/target/macos_aarch64/code.rs:232`) issues exactly one `write` and returns the
  bytes actually written.
- Correct siblings (advance-and-loop on short counts): `lower_stdout_drain`
  (`io_helpers.rs:41-49`); `lower_fs_write_all_helper` loop
  (`fs_helpers_io.rs:719-740`, `branch_le` → error on `<= 0`).
- Buffering feature: plan-14 (`fs::setBuffered`), memory note `plan-14-io-buffering`.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

```
IMPORT fs
FUNC main AS Integer
  RES f AS File = fs::createFile("/tmp/out.bin")
  fs::setBuffered(f, TRUE)
  fs::writeAll(f, strings::repeat("x", 100000))   # >= FILE_BUFFER_CAPACITY (4096)
  fs::close(f)
  RETURN 0
END FUNC
```

Point the output at a nearly-full filesystem (a small `tmpfs`), or at a pipe/FIFO whose
reader consumes slowly, so `write()` returns a short count.

- Observed: `writeAll` returns OK; the file/stream is missing the tail bytes that the
  single `write()` did not transfer.
- Expected: either all bytes are written, or `writeAll` raises `ErrOutput`.

Contrast (works today): the same write **without** `setBuffered` goes through
`lower_fs_write_all_helper`'s loop, which advances the cursor on a short count and
retries — so unbuffered large writes are complete.

## Root Cause

The two direct-write shortcuts in `emit_append_to_file_buffer` bypass the buffer for a
chunk that would not fit, but they emit a single `write` and check only for a **negative**
return (`branch_lt`). A `0 < n < len` return falls through to the `appended`/success
label. The unbuffered write helpers use `branch_le` (treat `<= 0` as error) and an
advance-and-loop cursor; the buffered shortcuts were written without that loop.

## Goal

- Both direct-write shortcuts write the full chunk (loop on short counts) or raise
  `ErrOutput`.
- A buffered large write to a filling disk / slow pipe never reports success with
  bytes unwritten.

### Non-goals (must NOT change)

- The buffering fast path for chunks that fit in the buffer.
- Unbuffered write helpers (already correct).
- The `ErrOutput` code / error surface.

## Blast Radius

- `emit_append_to_file_buffer` alloc-failed path (`:110-120`) and large-chunk path
  (`:139-149`) — fixed here.
- `lower_io_write_helper` direct/newline/oversized/alloc-fallback paths and
  `lower_io_read_line_helper` prompt write (`io_helpers.rs`) — fixed here (same one-shot
  `write` + `branch_lt` shape, default-path stdout).
- `lower_stdout_drain` (`io_helpers.rs:41-49`) — see the io/os LOW cluster for its
  `branch_lt`-on-0 spin; correct for short *positive* counts, so it is the fix template.
- `lower_fs_file_drain` (`fs_helpers_io.rs:41-48`) — its `branch_lt`-on-0 spin is in the
  fs LOW cluster; related but distinct.
- Unbuffered `fs` write loops — unaffected (already loop).

## Fix Design

Replace each single `write` with the advance-and-loop construct the unbuffered helpers
use: `compare_le → write_error`, `add cursor, n`, `subtract remaining, n`, `branch back`
until `remaining == 0`. Alternatively route the large chunk through
`lower_fs_file_drain`'s existing loop. The loop already exists and is tested for the
unbuffered path, so this is a reuse, not new logic.

## Phases

### Phase 1 — failing test

- [x] Add a buffered large-write test against a bounded `tmpfs` (or a FIFO with a slow
      reader) asserting either full content or `ErrOutput`. Confirm it loses the tail
      today. (Implemented as `native_io_short_write_returns_do_not_truncate_output` in
      `tests/native_io_runtime.rs`, using a `write()` interposer that caps every call
      to 4096 bytes — deterministic short positive returns. The old binary truncated a
      300000-byte write to 4096 bytes and reported success; the new binary transfers
      all 300000.)

### Phase 2 — the fix

- [x] Convert both shortcuts to the advance-and-loop write. (Plus the default
      `io::print`/`io::write`/newline paths and the `io::input` prompt write — every
      single-`write` + `branch_lt` output site now loops.)

### Phase 3 — validation

- [ ] Regenerate goldens (native `.ncode`/`.nplan` for io.write/print/printError/
      writeError, fs.writeAll/writeAllBytes, and io.input helpers shift — run by the
      orchestrator via `scripts/test-accept.sh`).
- [x] Re-ran the reproduction: both output paths recover fully under capped writes
      (old: 4096 bytes truncated + exit 0; new: 300000 bytes + exit 0). Nonblocking
      pipe: old reports silent success (exit 0, 65536/300000 bytes), new raises
      ErrOutput. `cargo test --test native_io_runtime` = 19/19 green.

## Validation Plan

- Regression test(s): the bounded-fs buffered-write test.
- Runtime proof: buffered write to a filling `tmpfs` either completes or errors.
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

Two buffered-write shortcuts check only for a negative `write` return, so a short
positive count reads as success and the tail is dropped silently. The fix reuses the
unbuffered path's advance-and-loop; only buffered large writes to short-count handles
change behavior.

## Resolution

Fixed. Every output path that issued a single `write` and checked only for a negative
return (`branch_lt`) now advances a cursor / decrements a remaining count in a loop
until nothing remains, treating a non-positive return (`branch_le` on `0` or `-1`) as
`ErrOutput` — the same construct `lower_fs_write_all_helper` already used. This both
recovers all bytes on a short *positive* return and stops (rather than spinning or
silently succeeding) on `0`/`-1`.

Sites converted:

- `src/target/shared/code/fs_helpers_io.rs` — `emit_append_to_file_buffer`: the
  alloc-failed direct write and the larger-than-buffer direct write (labels
  `alloc_failed_loop`, `big_write_loop`). Fixes `fs::writeAll`/`fs::writeAllBytes`
  under `fs::setBuffered(TRUE)`.
- `src/target/shared/code/io_helpers.rs` — `emit_append_to_stdout_buffer` (both
  buffered fallback writes), `lower_io_write_helper` (the default unbuffered direct
  write and the trailing-newline write), and `lower_io_read_line_helper` (the
  `io::input` prompt write). Fixes the common `io::print`/`io::write`/`printError`/
  `writeError` stdout/stderr path — buffering is off by default, so this is the path
  most programs hit.

Cursor/remaining are held in vregs (`%v40`/`%v41`, `%v13`/`%v14`, `%v41`/`%v42`), so
the register allocator spills and reloads them across each `bl write` — the
pointer/count are never read from a caller-saved register (compiler.md register
lifetimes). The empty-payload case is preserved: the loop checks `remaining == 0`
before the first write, so a zero-length `io::write("")` still succeeds without a
spurious `write(fd, ptr, 0)`.

### Runtime proof (macOS aarch64)

A `write()` interposer capping every call to 4096 bytes forces deterministic short
positive returns. Built the same source with the pre-fix compiler (a `git worktree`
at HEAD, without the working-tree edits) and the fixed compiler:

| Path | Pre-fix | Post-fix |
|------|---------|----------|
| `fs::writeAll` buffered, 300000 B → file | 4096 B, exit 0 (silent truncation) | 300000 B, exit 0 |
| `io::write` 300000 B → stdout | 4096 B, exit 0 (silent truncation) | 300000 B, exit 0 |
| `io::write` 300000 B → nonblocking pipe (no drain) | exit 0 (silent success, 65536 B) | ErrOutput 77020002, exit 255 |

Regression test: `native_io_short_write_returns_do_not_truncate_output`
(`tests/native_io_runtime.rs`, macOS-gated — linux-x86_64 issues a raw `write`
syscall no libc interposer can hook). Full `native_io_runtime` suite = 19/19 green.

### Relationship to bug-62

No overlap and no rework of EINTR. bug-62 owns the `write()`-returns-`0` drain spin in
`lower_stdout_drain`/`lower_fs_file_drain` (which still use `branch_lt`, treating `0`
as retry) and EINTR retry. The loops added here deliberately treat `0` and `-1` as
`ErrOutput` (`branch_le`), matching `lower_fs_write_all_helper`; they neither spin on
`0` nor retry EINTR, so they leave bug-62's scope untouched.
