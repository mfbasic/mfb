# plan-14-C: `fs::` per-handle read buffering

Last updated: 2026-07-05
Effort: medium

Part **C** of plan-14. Where **A** buffers stdout writes and **B** buffers `fs::`
writable handles, **C** adds a **read** buffer to the `fs::` `File` handle so
line/byte reads stop re-reading the file. Unlike A/B this is **transparent and
always on** (read buffering cannot lose or reorder data, so there is no opt-in
switch and no new user surface): the observable behavior of every program is
byte-identical, only far fewer syscalls and no quadratic blow-up.

The single behavioral outcome: a `WHILE NOT fs::eof(f)` / `fs::readLine(f)` loop
over an N-line file runs in **O(N)** with ~one `read()` per buffer-full, instead of
today's **O(N²)** (each `fs::readLine` seeks to EOF, reads the *entire remaining
file* to find the next newline, then seeks back). This is what actually fixes the
`io read` benchmark (~7 s → near C/Python); note plan-15 does **not** — plan-15
buffers **stdin** (`io::readLine` on `fd 0`), while the benchmark reads a **file**
via `fs::readLine`.

- **Depends on:** plan-14-B (reuses the per-`File`-handle buffer machinery — the
  buffer field set on the `File` resource layout; §4.5). Landable after B; the
  read buffer is a second field set on the same handle.
- **Spec/design:** overview §4.5 (per-handle buffer machinery it reuses) + §C-design below.

It complements:

- `./mfb spec io` / `src/docs/man/builtins/fs/{readLine,readByte,readChar,eof}.txt`
  (the line/byte read + EOF contract, which must stay byte-identical).
- `./mfb spec language memory-semantics` (unchanged — no value/copy/move impact).

## 1. Goal

- Replace the current O(N²) `fs::readLine` (`lower_fs_read_line_helper`,
  `src/target/shared/code/fs_helpers_io.rs:902`) with a buffered reader that serves
  lines from a per-handle read buffer, refilling with one block `read()` when the
  buffer is exhausted. Result: `fs::readLine` is O(1) amortized per line, O(N) per
  file.
- Route the other incremental reads (`fs::readByte`, `fs::readChar`) through the
  same read buffer.
- Make `fs::eof` buffer-aware: not-EOF while unconsumed buffered bytes remain.
- **Byte-identical observable behavior**: same lines (incl. CRLF trimming), same
  bytes, same EOF point, same errors — for every program, with no source change.
- Runtime: the `io read` benchmark drops from ~7 s to the low-ms range (near C/Python).

### Non-goals (explicit constraints)

- **No new user surface.** Read buffering is transparent/always-on; there is no
  `fs::setReadBuffered`. (Contrast A/B, which are opt-in because *write* buffering
  changes crash-visibility.)
- **stdin is out of scope** — `io::readLine`/`input`/`readChar`/`readByte` on `fd 0`
  are plan-15 (broadcast reader). C touches only `fs::` `File` handles.
- **Whole-file reads unchanged** — `fs::readText`/`readBytes` already read everything
  in one pass; they do not use and do not need the read buffer.
- No change to value/copy/move semantics, layout of user values, or the native ABI.
  The read buffer lives on the `File` resource, exactly like B's write buffer.

## 2. Current State

- `fs::readLine` (`fs_helpers_io.rs:902`) per call: `lseek(CUR)`→`start`,
  `lseek(END)`→`end`, `length = end-start` (**all remaining bytes**), arena-alloc a
  temp of `length`, **read the whole remaining file**, scan for `\n`, then
  `lseek(start+consumed)` back. → O(remaining) per line → **O(N²)** per file. This is
  the `io read` benchmark's cost (benchmark uses `readLinesFile` → `fs::readLine`,
  `benchmark/mfb/src/main.mfb:115`).
- The `File` resource layout carries `FILE_OFFSET_CLOSED`, `FILE_OFFSET_FD`
  (`fs_helpers_io.rs`). plan-14-B adds a write buffer field set (ptr/fill/enabled) to
  the same layout (overview §4.5) — C adds a **read** field set beside it.
- `fs::eof` today reflects the raw fd position vs size (seekable); it does not know
  about buffered read-ahead.

## 3. C-Design — the per-handle read buffer

Add a **read-buffer field set** to the `File` resource layout (beside B's write
buffer): `read_ptr` (arena block, lazily allocated on first incremental read),
`read_cap` (block size, e.g. 16 KiB), `read_pos` (next unconsumed byte offset),
`read_fill` (valid bytes in the block), and an `at_eof` flag (underlying `read()`
returned 0). All fields are per handle, lock-free (a `File` is single-owner).

**Buffered `readLine`:** scan `read_ptr[read_pos .. read_fill]` for `\n`. If found,
return the line (CRLF-trimmed exactly as today) and advance `read_pos` past it. If
the buffer is exhausted without a newline, copy the partial line into a growing
result (arena), then **refill**: `read(fd, read_ptr, read_cap)` → set `read_fill`,
`read_pos=0`; `0` bytes sets `at_eof` and returns the trailing partial line (or the
EOF error when nothing was pending), matching today's `eof_error`/no-newline
behavior. The logical file position is `raw_fd_pos - (read_fill - read_pos)`.

**`fs::eof`:** returns FALSE while `read_pos < read_fill` (unconsumed bytes buffered);
otherwise it reflects the underlying stream (refill-and-peek, or the `at_eof` flag).
This keeps `WHILE NOT fs::eof(f)` exactly correct across the buffer boundary.

**Seek / write reconciliation (correctness core).** The OS fd position runs *ahead*
of the logical read position by `read_fill - read_pos` unconsumed bytes. Any
operation that observes or moves the true fd position — `fs::seek`/`fs::position`,
or a *write* to a read+write handle — must first **reconcile**: `lseek(fd, -(read_fill
- read_pos), CUR)` to rewind the unconsumed read-ahead, then **invalidate** the read
buffer (`read_pos=read_fill=0`, clear `at_eof`). After that the fd is at the logical
position and the existing seek/write path runs unchanged. `readText`/`readBytes`
(whole-file) also reconcile+invalidate first so they see the true position.

**Interaction with B's write buffer.** On a read+write handle, a mode switch flushes
the *write* buffer (B) and reconciles+invalidates the *read* buffer (C) — a handle is
never simultaneously mid-read-ahead and mid-write-buffer against the same fd offset.
Read-only handles (the benchmark and the common case) never hit the write side.

## Layout / ABI Impact

Adds the read-buffer field set to the **`File` resource** runtime layout only (as B
does for writes) — coordinate the field offsets with B so the two field sets don't
overlap. No change to `ARENA_STATE` (unlike A/plan-15), no change to any user value
layout, copy/transfer, or the native ABI. Programs that never call an incremental
`fs::` read produce byte-identical output and identical native code paths for the
non-read helpers. `fs::readLine`/`readByte`/`readChar`/`eof` native code changes (new
helpers), but their **observable** results are byte-identical.

## Phases

### Phase C1 — read buffer + buffered `fs::readLine` + `fs::eof`

The core O(N²)→O(N) fix. Landable alone (readByte/readChar can still use the old path
transiently, or route in C2).

- [ ] Add the read-buffer field set (ptr/cap/pos/fill/at_eof) to the `File` resource
      layout, beside B's write buffer (`fs_helpers_io.rs`, the `File` layout constants).
- [ ] Rewrite `lower_fs_read_line_helper` (`fs_helpers_io.rs:902`) to serve from the
      read buffer + refill, dropping the seek-to-EOF/read-whole-remaining/seek-back loop.
      Preserve exact line semantics (LF split, trailing CRLF trim, final-partial-line,
      empty-file/EOF error).
- [ ] Make `fs::eof` buffer-aware (not-EOF while `read_pos < read_fill`).
- [ ] Wire read-buffer teardown into the `File` resource drop/`fs::close` (free the
      block; no flush needed — reads discard cleanly).
- [ ] Tests: `tests/func_fs_readLine_*` and `tests/func_fs_eof_*` extended for
      multi-line files that span a buffer boundary (file > `read_cap`), empty file,
      no-trailing-newline, and CRLF; `_valid/**` + `_invalid/**`.

Acceptance: a `WHILE NOT fs::eof(f)`/`fs::readLine` loop over a file larger than
`read_cap` returns byte-identical lines to today; `strace`/`dtruss` shows ~1 `read`
per `read_cap` (not per line); the `io read` benchmark drops from ~7 s to low-ms; the
full acceptance suite is byte-identical (golden diff = ∅ for existing fs tests).
Commit: —

### Phase C2 — `readByte`/`readChar` + seek/write reconciliation

Route the remaining incremental reads through the buffer and make seek/write correct.

- [ ] Route `fs::readByte`/`fs::readChar` through the read buffer (refill on exhaustion).
- [ ] Add reconcile-then-invalidate on `fs::seek`/`fs::position` and on the whole-file
      `fs::readText`/`readBytes` paths, and on a *write* to a read+write handle
      (compose with B's write-buffer flush).
- [ ] Tests: interleaved `readLine`+`fs::seek`+`readByte` on one handle returns the
      same bytes as an unbuffered handle; a read-then-write-then-read handle stays
      byte-correct; `tests/func_fs_readByte_*`/`func_fs_readChar_*` across a buffer
      boundary.

Acceptance: mixed read/seek/write on one handle is byte-identical to the unbuffered
reference; incremental byte/char reads collapse to ~1 `read` per `read_cap`; full
suite + acceptance pass.
Commit: —

## Validation Plan

- Function tests: `tests/func_fs_readLine_*`, `func_fs_eof_*`, `func_fs_readByte_*`,
  `func_fs_readChar_*` — `_valid/**` + `_invalid/**`, spanning a buffer boundary.
- Runtime proof: (a) `io read` benchmark ~7 s → low-ms (the O(N²)→O(N) win);
  (b) byte-identical lines/bytes/EOF vs the unbuffered reference on multi-line,
  empty, no-trailing-newline, CRLF, and >`read_cap` files; (c) `strace`/`dtruss`
  syscall collapse (~1 `read` per block); (d) mixed read/seek/write byte-correctness.
- Doc sync: update `fs/{readLine,readByte,readChar,eof}.txt` (note buffering is
  transparent — no behavior change), `fs/package.txt` (per-handle read buffer beside
  the write buffer). No new builtins, so no new man pages / builtin-list entries.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` — golden
  diff must be ∅ for existing fs fixtures (transparency guard), and the new fs read
  tests pass.

## Open Decisions

- `read_cap` block size — recommend 16 KiB (matches typical libc `BUFSIZ`×; big
  enough to collapse syscalls, small enough to not bloat every `File`). (§3)
- Lazy vs eager buffer allocation — recommend lazy (allocate `read_ptr` on the first
  incremental read), so a handle only used for whole-file reads pays nothing. (§3)

## Summary

The engineering risk is the seek/write **reconciliation** (the fd runs ahead of the
logical read position by the unconsumed buffer) and preserving `fs::readLine`'s exact
line/CRLF/EOF semantics — both bounded by the byte-identity acceptance guard. It
reuses plan-14-B's per-`File`-handle buffer machinery, touches no `ARENA_STATE` and no
user-value layout, and is the piece that actually fixes the `io read` benchmark
(distinct from plan-15's stdin reader).
