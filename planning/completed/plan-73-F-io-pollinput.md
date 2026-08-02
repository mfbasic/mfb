# plan-73-F: io::pollInput timeout migration

Last updated: 2026-08-01
Effort: small (1h–3h)
Depends on: plan-73-A (convention, shared sentinel, canonical spec section).

Added during plan-73-E's Phase 1 conformance audit. The audit is what this letter
exists to satisfy: `io::pollInput` is a waiting built-in that the original A–D
split **missed**, and it carried the exact *inverted* pre-plan-73 convention the
whole plan exists to remove — negative meant "block forever", an omitted argument
was padded with `0` (a non-blocking check), and the value went straight to
`poll(2)`. Leaving it would falsify the canonical spec text plan-73-A itself wrote
("Every built-in that can wait … interprets it identically … There is no
per-package variation in the meaning of the value"). Per the follow-plan rule for
a prerequisite/function no letter covers, it is landed as a new (append-only)
letter with the edge added to A's graph.

References:

- plan-73-A — the convention + canonical spec section + shared sentinel.
- `.ai/compiler.md`, `.ai/man_template.md`, `.ai/specifications.md`.
- Analog: `net::poll` (`src/target/shared/code/net/poll.rs:lower_net_poll_helper`)
  — the sibling readiness-query-over-`poll()` whose sentinel→block / `<0`→invalid
  / clamp-to-`INT_MAX` structure this mirrors exactly.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-73-A complete (convention + `TIMEOUT_UNBOUNDED_SENTINEL`) | `grep -n TIMEOUT_UNBOUNDED_SENTINEL src/target/shared/code/error_constants.rs` | MET |
| `io::pollInput` is a readiness query (returns `Boolean`) | `grep -n POLL_INPUT src/builtins/io.rs` (return type `Boolean`, arity `(0,1)`) | MET |

## 1. Goal

`io::pollInput([timeoutMs AS Integer]) AS Boolean` obeys the plan-73 convention as
a **readiness query**:

- **omit** → block until standard input is ready, then `TRUE` (padded with the
  unbounded sentinel → `poll(2)` with a `-1` timeout).
- **`0`** → one immediate readiness check (the old omitted behavior).
- **`> 0`** → wait up to that many ms, clamped to `2147483647` (poll takes a C
  `int`; a bit-31 value must not be misread as a block/negative timeout, bug-239).
- **`< 0`** → `ErrInvalidArgument` (77050002).

End-of-input still counts as ready (a `TRUE` promises the next read will not block,
not that it will succeed); the EINTR-retry and broadcast-log fast path are
unchanged.

### Non-goals

- No change to the readiness semantics, the stdin broadcast log, EINTR handling,
  or the `ErrInput` failure path — only the `timeoutMs` value meaning.

## 2. Current State (pre-F)

- Descriptor: `src/target/shared/code/builder_values.rs:lower_runtime_helper_call`
  padded an omitted `io.pollInput` timeout with `0` (a non-blocking check).
- Codegen: `src/target/shared/code/io_stdin.rs:lower_io_poll_input_helper` loaded
  the raw `timeoutMs` and passed it straight to `poll(2)` — POSIX semantics:
  negative = block forever, `0` = immediate. The exact inversion plan-73 removes.
- Man: `src/docs/man/builtins/io/pollInput.md` documented "negative waits forever,
  `0` returns immediately … defaults to `0` when omitted".

## Phases

> Keep checkboxes current in-commit; fill `Commit:`; unticked = NOT DONE.

### Phase 1 — descriptor + codegen + docs + fixture

- [x] Descriptor: pad an omitted `io.pollInput` timeout with
      `TIMEOUT_UNBOUNDED_SENTINEL` instead of `0`
      (`builder_values.rs:lower_runtime_helper_call`). — DONE.
- [x] Codegen: normalize `timeoutMs` up front in
      `io_stdin.rs:lower_io_poll_input_helper` — sentinel → `poll(-1)` (block);
      `< 0` → `ErrInvalidArgument` (new `poll_invalid` tail, placed before
      `poll_error` and terminated with a branch to `done` so `poll_error` still
      falls through, byte-preserving); `> 0` clamped to `INT_MAX`; the normalized
      value is stashed back to the timeout stack slot so the EINTR-retry `os_poll`
      loop reloads it. `_mfb_str_error_invalid_argument` is registered
      unconditionally (`data_objects.rs:87`), so the new relocation resolves with
      no data-object gate change. — DONE.
- [x] Man: rewrite `src/docs/man/builtins/io/pollInput.md` to the convention (omit
      = block, `0` = immediate, `> 0` bounded, `< 0` = `ErrInvalidArgument`; add
      the `ErrInvalidArgument` error row; cite `mfb spec language
      builtin-functions`; fix the three examples that used the old meanings). — DONE.
- [x] Spec: add `io::pollInput` to the canonical §18.4 readiness-query bullet and
      the "Conforming functions" list. — DONE.
- [x] Fixture: migrate `tests/rt-behavior/io/func_io_pollInput_valid` — the old
      `IF FALSE / pollInput(-1)` "waitForever" block is no longer a valid form;
      the fixture now exercises omit/`0`/`> 0` (all `TRUE` at EOF) and a runtime
      TRAP proving `pollInput(-1)` → `77050002`. Regenerated its `.ast`/`.ir`/
      `build.log`; emptied the `.run` marker (contents are never compared). — DONE.
- [x] Goldens: regenerate `byte-identity/io` `.ncodesum` for all five targets
      (macos-aarch64, linux-{aarch64,x86_64,riscv64}, windows-x86_64). — DONE:
      determinism check (N=3/target) shows uniq=1 and MATCH for every target.

Acceptance: `io::pollInput` matches the §18.4 table on omit/`0`/`> 0`/`< 0`;
runtime-proven on macOS (`TRUE TRUE TRUE` for omit/`0`/`1`, `77050002` for `-1`);
man + spec citations green; `byte-identity/io` regenerated and deterministic;
`cargo build` warning-free. — MET. Commit: 99702c21c

## Corrections

- **F-C1 (why this letter exists).** The A–D split enumerated its function list as
  `net::* / tls::* / audio::{poll,read} / thread::{send,receive,transfer,accept}`
  and never included `io::pollInput`, even though it is a waiting built-in and
  carried the inverted convention. Found by plan-73-E Phase 1's authoritative
  descriptor sweep (`grep -rn '"timeoutMs"|"ms"|"timeout"' src/builtins/*.rs`),
  which is the exhaustive check the E audit is for. Routed here as a new letter
  rather than patched silently in E.

## Summary

Closes the one waiting built-in the original plan-73 split missed. `io::pollInput`
now blocks on omit, checks immediately on `0`, bounds on `> 0`, and rejects
negatives — the same one convention every other waiting built-in follows, making
the canonical spec's "no per-package variation" claim true across the whole tree.
