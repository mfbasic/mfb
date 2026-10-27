# bug-1121: `thread::transfer` of a buffered `fs::File` drops the bytes still in its buffer

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: MED
Class: Runtime (data loss)

Status: Open
Regression Test: —

`mfb man fs isBuffered` says transferring a buffered handle resets it to
unbuffered. The reset also throws away whatever the sender had written but not
yet flushed.

Repro (plan-125 reviewer probe `proj_p3`): `fs::setBuffered(f, TRUE)`,
`fs::writeAll(f, "pending-before-transfer\n")`, `thread::transfer` `f` to a worker
that writes `"worker\n"` and closes it; `fs::readText(path)` afterwards is
`"worker\n"`. The first line is lost with no error.

**The correct behavior a fix produces:** the transfer drains the buffer (as
`fs::close` and scope exit do) before handing the handle over, so the file holds
`"pending-before-transfer\nworker\n"`; a failed drain raises `ErrWriteFailed` at
the transfer.

References:

- Found by plan-125-D Phase 5, finding `fs-isBuffered` 2; probe
  `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-isBuffered/proj_p3`.
