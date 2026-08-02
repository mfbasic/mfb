# bug-410: `term::sync` present-write loop treats EINTR as give-up → permanent frame corruption on a mid-write signal; the `write_retry` label is dead

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (EINTR handling; partial-frame desync) + dead code

Status: FIXED
Regression Test: tests/ — a TUI present interrupted by a signal mid-`write` must
re-issue the write and paint the full frame (timing harness or a fault-injected
write).

## STATUS: FIXED

Reproduced statically as documented (no deterministic runtime repro exists
without a fault-injection harness): the `write_retry` label had **no incoming
branch** and `:1144` `branch_le(write_done)` gave up on any negative return, and
`term.sync` imported **no** errno accessor on either the Linux or macOS plan —
so even a wired retry could not classify EINTR on a libc-`write` backend.

Two coupled parts, both landed together (single root cause, serialized):

1. **Codegen** (`term_grid.rs::emit_grid_present`): the hand-rolled present-write
   tail now routes the write result through the shared `emit_transfer_loop_tail`
   every sibling write loop uses — a positive count advances, a negative return
   is EINTR-retried at the loop top, and a 0 return for a nonzero write is the
   bug-62 hard give-up. The dead `write_retry` label is removed (the retry edge
   is the loop top). `write_done` remains the give-up target (this present has no
   error channel; `front == back` already claims the frame painted).
2. **Imports**: `term.sync` now imports the errno accessor on libc-`write`
   backends — `__errno_location` on Linux aarch64/riscv64 (gated `!raw_write`, so
   x86-64's raw `svc` write, which returns `-errno`, does not get a dead import)
   and `___error` on macOS. `symbols.rs` force-pulls `term.sync`'s import set
   whenever any `term::` helper is used, so `term::off`/auto-restore's reuse of
   the present helper is covered. Windows app-mode `term.sync` is a Win32 GUI
   redraw (`InvalidateRect`/`UpdateWindow`), not the console present loop, so it
   is untouched.

Regression tests (RED→GREEN): a codegen test drives `emit_grid_present` with the
accessor available and asserts the `cmp <ret>, EINTR; b.eq <write_loop>` retry
edge; per-backend import tests assert `__errno_location`/`___error` is present on
the libc backends and **absent** on x86-64 (raw write). Goldens regenerated:
`byte-identity/term` `.ncodesum` (5 targets) and `macos-app-mode-term`
`macos-aarch64.app.{ncode,nplan}`; front-end (`.ast`/`.ir`/`.nir`) byte-identical.

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
