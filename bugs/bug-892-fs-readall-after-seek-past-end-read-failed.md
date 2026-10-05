# bug-892: fs::readAll / fs::readAllBytes raise ErrReadFailed after fs::seek past end of file

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/fs/<new> (seek past end, then readAll/readAllBytes)

`fs::seek` is documented to allow a position past the end of the file, and `fs::eof` then reports
`TRUE`. A following `fs::readAll` or `fs::readAllBytes` raises `ErrReadFailed` ("Read operation
failed.") where it should return the empty value. The man page says that at end of input "no bytes
remain and the empty String is returned". A seek to exactly the end does return `""`, so only a
position strictly past the end hits the bug. This affects `fs::openData` files and disk files alike.

The correct behavior: when the read position is at or past the end, `readAll` returns `""` and
`readAllBytes` returns `[]`. `ErrReadFailed` stays reserved for a handle that cannot be measured or
repositioned.

References:

- `src/docs/spec/stdlib/23_fs-memory-files.md` §"fs::seek": "A position past the end is allowed."
- `./mfb man fs readAll`: "When file is already at end of input, no bytes remain and the empty String
  is returned." `fs::eof` agrees that the file is at end (prints `TRUE`).
- Found during spec-hardening batch B08.

## Failing Reproduction

`/tmp/hardening/B08/repro-seek/src/main.mfb`:

```
IMPORT io
IMPORT fs

FUNC main() AS Nothing
  RES f AS fs::File = fs::openData("abcdef", "r")
  fs::seek(f, 10)
  io::print("eof=" & toString(fs::eof(f)))
  io::print("readAll=[" & fs::readAll(f) & "]")
END FUNC
```

```
cd /tmp/hardening/B08/repro-seek && /tmp/hardening/mfb build && ./build/repro-seek.out
```

- Observed: `eof=TRUE`, then `Error: 7-702-0001` / `Read operation failed.` (exit 255)
- Expected: `eof=TRUE`, `readAll=[]`

Contrast cases (`/tmp/hardening/B08/md2`):

- `fs::seek(f, 6)` (exactly at end) then `fs::readAll` → `[]` ✓
- the same with `fs::readAllBytes` → `ErrReadFailed` ✗
- a disk file from `fs::open(path, "rw")` with the same seek → `ErrReadFailed` ✗. So `openData` is
  not the cause.
- After the same seek, `fs::readLine` → `ErrEndOfFile` and `fs::readBytesAt` work. Writing at 10
  extends the file with a zero-filled hole (`od -c` shows `a b c d e f \0 \0 \0 \0 Z`) ✓

## Root Cause

`src/codegen/builtins/fs/gen_read_write.rs:lower_fs_read_all_helper` measures the remainder with
three seeks (`start` = SEEK_CUR, `end` = SEEK_END, then back to `start`), and then does
`compare_registers(end, start); branch_lt(seek_error)`. That guard exists to catch "a file that shrinks
while it is read", but `end < start` is also the ordinary state after a legal seek past the end, so the
guard sends that case to `ErrReadFailed`. `lower_fs_read_all_bytes_helper` (same file) has the same
`branch_lt(&seek_error)` sequence.

## Goal

- After `fs::seek(f, p)` with `p > size`, `fs::readAll(f)` returns `""` and `fs::readAllBytes(f)`
  returns `[]`.
- The repro prints `readAll=[]`.

### Non-goals (must NOT change)

- `ErrReadFailed` for non-seekable handles (a FIFO; `/tmp/hardening/B08/md4` shows `fs::seek` on a
  FIFO raising 77020001, which is correct).
- The "file shrank during the read" detection for the bytes actually being read.

## Blast Radius

- `lower_fs_read_all_helper`, `lower_fs_read_all_bytes_helper`: fixed by this bug.
- `lower_fs_eof_helper` (same file) also seeks CUR/END, but it only checks the host return codes.
  It has no `end < start` guard and correctly reports `TRUE` past the end.
