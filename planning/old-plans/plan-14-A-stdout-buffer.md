# plan-14-A: Opt-in stdout buffering

Last updated: 2026-07-05 (reference refresh — see overview banner)
Effort: medium

Part **A** of plan-14 (Opt-In Output Buffering). Delivers the per-thread stdout buffer, its control
builtins, and every drain hook — the core of the feature. Shared design (goal, non-goals, runtime
state, drain-hook rationale) lives in the overview:
[plan-14-io-buffering.md](plan-14-io-buffering.md).

- **Depends on:** nothing — land first. (Coordinate the `ARENA_STATE_SIZE` bump with plan-15's
  stdin words if landing together.)
- **Blocks:** plan-14-B reuses the buffer machinery for the `fs::` per-handle mirror.
- **Spec/design:** overview §4.1–§4.4.

## Phases

### Phase A1 — Builtins + runtime buffer, default off, with full drain hooks

The stdout buffer, its controls, and every drain hook — with zero cost on the default path.

- [ ] Add `io::isBuffered` / `io::setBuffered` builtins and give `io::flush` real teeth (`src/builtins/io.rs`; §4.2): `IS_BUFFERED => Some("Boolean")`, `SET_BUFFERED => Some("Nothing")`, arity 0/1.
- [ ] Reserve the three `ARENA_STATE` words `OUT_PTR`/`OUT_FILLED`/`OUT_ENABLED` (appended after `ARENA_CARVE_SIZE_OFFSET` in `src/target/shared/code/error_constants.rs`), bump `ARENA_STATE_SIZE`, and zero-init where the block is already zeroed (§4.1) — coordinate the size bump with plan-15's stdin words (one combined change; the derived `ENTRY_ARGC/ARGV` + any hardcoded size move with it).
- [ ] Add the one-branch buffered-write prologue to `lower_io_write_helper` (`src/target/shared/code/io_helpers.rs:3`): `OUT_ENABLED == 0` falls straight into today's direct-`write` path; enabled copies into the buffer / drains on overflow (§4.1). Route `io::print` through the same helper.
- [ ] Wire all four drain hooks (§4.3): `_mfb_shutdown` exit, before every stdin read, buffer-full, and the `setBuffered(FALSE)` transition.
- [ ] Tests: `tests/func_io_isBuffered_*`, `tests/func_io_setBuffered_*`, `tests/func_io_flush_*` (`_valid/**` + `_invalid/**`).

Acceptance: with buffering off, every existing test/golden/acceptance result is byte-identical (the default path is unchanged); with buffering on, a clean run produces identical bytes to the unbuffered run, and a print loop issues ~1 `write` per 4 KiB under syscall inspection.
Commit: —

### Phase A2 — Prompt + read-flush coverage

Never leave a buffered prompt unseen while the program blocks on input.

- [ ] Make every stdin read (`io::readLine`/`input`/`readChar`/`readByte`) drain the stdout buffer before blocking (§4.3 hook 2).
- [ ] `io::input(prompt)` auto-flushes both pending buffered output *and* its own prompt argument before reading.
- [ ] Tests: interactive prompt programs in both the `io::print`+`io::readLine` and the `io::input` forms.

Acceptance: interactive prompt programs (both forms) behave identically with buffering on and off — the prompt always appears before the read blocks.
Commit: —

### Phase A3 — Threading + exit/signal coverage

Per-thread buffers, each draining on its own exit path.

- [ ] Each thread's stdout buffer (per-arena `x19` state) drains in its own `_mfb_shutdown`/exit path (§4.4).
- [ ] SIGINT/SIGTERM drains the main buffer via `_mfb_shutdown` before `_exit(128+signo)`.
- [ ] Document the hard-crash-loses-unflushed-bytes caveat.

Acceptance: a buffered multi-thread program and a SIGINT-interrupted buffered program lose no already-`flush`ed output; the hard-crash limitation is documented.
Commit: —
