# bug-208: buffered stdout drain partial-write cursor makes the append path write past the 4 KiB buffer (heap OOB)

Last updated: 2026-07-14
Effort: medium (1h–2h)
Severity: MEDIUM
Class: memory-safety

Status: Fixed (2026-07-15) — `lower_stdout_drain` captures the buffer base in a
dedicated vreg and, on the partial-write error path, slides the unflushed tail
back down to the base (keeping `OUT_PTR` = base) instead of advancing `OUT_PTR`
into the middle of the buffer. The append path's "`OUT_PTR` is the fixed 4 KiB
base" invariant now holds, so a subsequent buffered write can no longer copy past
the buffer end. A retried flush still resumes correctly (bug-97 preserved).
Regression Test: normal buffered stdout verified at runtime (5000-byte output
crossing the 4 KiB buffer is emitted correctly). The partial-write-then-error
path is not directly runtime-triggerable (needs a stalled/ENOSPC fd mid-drain).

After a partial-write-then-error drain, `lower_stdout_drain` advances
`ARENA_OUT_PTR` to a resume cursor, but `emit_append_to_stdout_buffer` treats
`ARENA_OUT_PTR` as the fixed 4 KiB buffer **base**. A later buffered write copies
at `base+k+filled` and runs past the buffer end — a heap OOB write, and a
subsequent drain then reads OOB too. (Distinct from bug-97, which only fixed
double-send; that fix introduced the base/cursor conflict here.)

## Failing Reproduction

`io::setBuffered(TRUE)`, then a drain where `write` lands N>0 bytes and the next
`write` hard-errors (e.g. stdout redirected to a filling disk → ENOSPC). The err
path stores `OUT_PTR=base+k`, `OUT_FILLED=remaining`; the next `io::print` (or the
drain swallowed inside `io::readByte/readChar/readLine`, which ignore the drain
result at `:1159`/`:1336`/`:1725`) copies `len` bytes at `base+k(+remaining)`,
exceeding the `OUT_BUFFER_CAPACITY` (4096) allocation.

## Root Cause

`src/target/shared/code/io_helpers.rs:87-88` (`lower_stdout_drain` err path
advances `ARENA_OUT_PTR`) vs `:226-256` (`emit_append_to_stdout_buffer` `fits`
treats `ARENA_OUT_PTR` as the fixed buffer base).

## Non-goals

- Do not reintroduce the bug-97 double-send.

## Blast Radius

- `io_helpers.rs` buffered-stdout drain + append paths.

## Fix Design

On the drain error path, memmove the unflushed tail down to the buffer base and
keep `OUT_PTR` fixed (or store a separate base vs. cursor field) so the append
path's base invariant holds.
