# bug-410: `term::sync` present-write loop treats EINTR as give-up → permanent frame corruption on a mid-write signal; the `write_retry` label is dead

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (EINTR handling; partial-frame desync) + dead code

Status: Open
Regression Test: tests/ — a TUI present interrupted by a signal mid-`write` must
re-issue the write and paint the full frame (timing harness or a fault-injected
write).

`term::sync` presents a diffed frame as one batched `write(1, outbuf, remaining)`
and copies each emitted cell back→front (`front = back`) **before** the write
completes (`src/target/shared/code/term_grid.rs:1066-1069`). The write loop
(`term_grid.rs:1125-1150`) advances on positive short writes but treats **any
negative return as give-up**:

```
cmp wres, 0
branch_le(write_done)     // term_grid.rs:1144 — negative (EINTR) exits the loop
```

A negative return is `EINTR`. MFBASIC's signal handlers are **not** `SA_RESTART`
(the entire bug-314 cluster exists because read/write/poll return EINTR). So if a
signal is delivered during the present `write()` — `SIGWINCH` on a terminal resize
(which itself forces a full `dirty` repaint, i.e. the largest, most interruptible
write), `SIGCHLD`, or the console `SIGINT`/`SIGTERM` handler — the loop aborts with
`remaining` bytes unsent while `front == back` claims the whole frame was painted.
The next `term::sync` diffs to nothing and never repairs the missing cells —
**permanent frame corruption** until those cells happen to change.

This is exactly the failure mode bug-314 H3 was written to prevent. Its completion
notes assert "retry EINTR … `term::sync` repairs partial writes", but the landed
code retries only **positive short writes**, not EINTR. The inline comment
(term_grid.rs:1141-1143) even says "A negative return is EINTR (retry) or a genuine
failure," and the `write_retry` label (declared `:1122`, placed `:1132`) has **no
incoming branch** — dead code that is the vestige of the intended EINTR re-issue.

Every sibling write loop routes negatives through `emit_eintr_retry_or_error` /
`emit_transfer_loop_tail` (which DO retry EINTR) — e.g. `io_stdout.rs` drain/write;
only this hand-rolled present loop omits it.

References:

- `src/target/shared/code/term_grid.rs:1144` (give-up `branch_le` on `cmp wres,0`),
  `:1122`/`:1132` (dead `write_retry` label), `:1066-1069` (`front=back` before the
  write completes).
- bug-314 H3 (`bugs/completed/bug-314-…md`) — claims EINTR retry, but only
  positive short writes are retried. Contrast `io_stdout.rs` `emit_eintr_retry_or_error`.
  Found during goal-07.

## Failing Reproduction

No deterministic repro run (triggering a signal inside the present `write()`
syscall needs a timing/fault-injection harness). Static evidence: `write_retry` has
no incoming branch (grep), and `:1144` `branch_le(write_done)` exits on any
negative, unlike the sibling EINTR-retrying loops.

- Observed: on EINTR mid-present, the loop exits with bytes unsent; `front=back`
  suppresses repair → corrupt frame persists.
- Expected: EINTR re-issues the `write` (branch to `write_retry`) and the full
  frame is painted.

## Root Cause

The present-write loop discards a negative (EINTR) return instead of retrying;
`front=back` is committed before the write is confirmed complete, so a truncated
present is never repaired.

## Goal

- The `term::sync` present write retries on `EINTR` (re-issue via the existing
  `write_retry` label), so a signal mid-present never leaves the frame partially
  painted.

### Non-goals (must NOT change)

- The 0-return-is-failure guard (bug-62) for a genuine no-progress write; only the
  negative/EINTR case must retry.

## Blast Radius

- `src/target/shared/code/term_grid.rs:1144` — route EINTR (`wres < 0` &&
  `errno == EINTR`) to `write_retry` instead of `write_done`; wire the dead label.
- `term::off` (calls `sync`) inherits the fix.
