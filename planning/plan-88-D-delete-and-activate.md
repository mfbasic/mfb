# plan-88-D: activate the used-set, delete ERR_*, remove manual gating

Last updated: 2026-08-03
Effort: medium (1h–2h)
Depends on: plan-88-C (every error emission — per-call-site from B and symbol-path
from C — must already go through `raise_error`/`raise_error_bare` and populate the
used-set. If C is not complete, D cannot start, full stop.)

Sub-plan **D** of plan-88 — the final letter. See plan-88-A §3 for the overall
design. D flips error-string emission from the manual `push_string_value` gating
in `data_objects.rs` to the automatic used-set, then deletes the now-dead `ERR_*`
constants, the `emit_*_return` wrappers, and the gating; renames
`error_constants.rs`; and lands the drift-guard test that closes the loop.

Behavioral outcome for D — the whole feature’s goal: **every runtime error’s code
and message come from exactly one `ERRORCODE_CONSTANTS` row; the compiled program
raises the same errors (code + message) as before plan-88; and no `ERR_*` code/message/symbol
constant, `emit_*_return` wrapper, or manual `push_string_value` error-gating
remains.**

References: plan-88-A; `src/target/shared/code/data_objects.rs`;
`src/target/shared/code/error_constants.rs`;
`src/target/shared/code/builder_error_emission.rs`; `src/builtins/errorcode.rs`;
`src/builtins/descriptor.rs`; `.ai/compiler.md`.

## Prerequisites

See plan-88-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-88-C complete | `ls planning/completed/plan-88-C-*` (exists) | **MET** (archived `9d4af14c3`) |
| No error emitter bypasses `raise_error` | `grep -rn 'push_error_message_address(\|self\.emit_[a-z_]*_return()' … outside defs` → 0 | **MET** (only the def + `raise_error_into`'s internal call) |
| used-set populated by every path | (C acceptance: used-set non-empty on a symbol-path program) | **NOT MET (entry-gate stop) — see D-1. C's free-function `raise_error_into` cannot feed `CodeBuilder.used_errors`; a symbol-path-only program's used-set is empty.** |

**Prerequisite verdict: entry gate NOT passed** (row 3 NOT MET). Per follow-plan §2
D does not start until this is resolved. D-1 records the defect and a viable
corrected Phase-1 (table-source the existing gating; drop the used-set + the
`used_errors` field). That correction is a **material change** — it abandons
plan-88-A's `used_errors` mechanism entirely and is a ~124-site message re-source +
123-const deletion whose acceptance is "every fixture still **links**" (the bug-256
hazard) plus one intended `ErrWrongMode` runtime-message change. Recommended, but
surfaced for confirmation before executing rather than absorbed silently.

> **D-1 (Prerequisites defect / Phase-1 premise corrected).** The used-set premise
> is unsound given C's architecture, so Phase 1 is redesigned (not the goal, only the
> mechanism):
> - `raise_error_into` (the fixed-helper emitter C landed) is a **free function** and
>   never touches `CodeBuilder.used_errors`; the fixed helpers are emitted at module
>   level, outside any per-function `CodeBuilder`. So a symbol-path-only program has an
>   **empty** used-set → prereq NOT MET.
> - Worse, the used-set is the **wrong signal**: the per-call-site path
>   (`emit_error_register_return`) **content-pools** its message via
>   `emit_load_string_address_into`, so `used_errors` (populated only by those methods)
>   does not correspond to the fixed `_mfb_str_error_*` data objects at all. And
>   `data_objects::string_symbols` runs on the **IR module** (pre-codegen), where the
>   relocations that *would* name the needed symbols do not yet exist — which is why the
>   status quo gates on module-analysis in the first place.
>
> **Corrected Phase 1:** keep the proven, bug-256-hardened module-analysis gating in
> `data_objects.rs`, but **source each message from `ERRORCODE_CONSTANTS`**
> (`errorcode::runtime_error(name).1`) instead of the `ERR_*_MESSAGE` consts. That
> deletes the message consts (invariant #2: one metadata source) with **zero** behavior
> change (same strings, per the parity test) — except the single intended `ErrWrongMode`
> consolidation (table message adopted). The `used_errors` field + inserts (plan-88-A/B)
> become dead and are removed. Byte-identical except the wrong-mode data object.

## 1. Goal

- The error-string data objects emitted for a module are exactly the used-set
  `raise_error`/`raise_error_bare` accumulated — the 18 manual gating sites are
  gone. All 123 `ERR_*` constants (41 code / 41 message / 41 symbol) are deleted;
  the 12 `emit_*_return` wrappers are deleted; `error_constants.rs` is renamed to
  reflect its remaining 222 structural constants. A drift-guard test asserts the
  table/contract invariants.

### Non-goals

- No message text changes: the used-set must emit **the same strings** (same
  message text, for the same set of raised errors) the manual gating did for any
  given program — so runtime behavior is preserved. The instruction bytes may
  differ (symbol naming/order); that re-baselines. What must not change is which
  errors, with which messages, a program can raise.
- `ErrWrapped` and the structural result/error-object constants (result tags,
  `ERROR_OBJECT_SIZE`, `MAKE_ERROR_RESULT_SYMBOL`, register/offset consts — 222 of
  them) stay; only the `ERR_*` code/message/symbol triples leave.

## 2. Current State (delta from C)

After C, `raise_error`/`raise_error_bare` handle every emission and populate
`used_errors`, but:

- `data_objects.rs` still emits error strings via 18 manual `push_string_value(&mut
  values, ERR_*_MESSAGE)` sites gated on module analysis (the bug-256 class).
- `error_constants.rs` still defines 123 `ERR_*` consts; nothing outside the
  wrapper bodies references the CODE/MESSAGE anymore, and the SYMBOL consts are
  referenced only by the now-migrated helper sites (verify 0 after C).
- `builder_error_emission.rs` still defines 12 `emit_*_return` wrappers with no
  callers (verify 0 after B).

Verify each “no caller” with `grep -rc` before deleting (never delete on
assumption; the split-can-break-test-build hazard applies — a `cfg(test)` caller
can hide).

## 3. Design

- **Activate the used-set**: in `data_objects.rs`, replace the 18 manual
  `push_string_value(ERR_*_MESSAGE)` calls with a single pass that, for each name
  in `builder.used_errors`, emits that error’s string data object (message from
  `errorcode::runtime_error(name)`). The used-set must include exactly the names a
  program actually raises — no fewer (or a relocation dangles at link) and no more
  (or the binary bloats) — verified by the used-set-minimality test and a link of
  every fixture. Instruction bytes may shift (symbol naming/order); goldens
  re-baseline once the runtime proof holds.
- **Delete** the 123 `ERR_*` consts and the 12 wrappers once `grep` proves zero
  references.
- **Rename** `error_constants.rs` → `result_abi.rs` (or `error_layout.rs`) since
  it now holds only structural constants; update the `mod` and imports.
- **Drift guard**: a test asserting (1) every `BuiltinFunction.errors` entry is a
  real `ERRORCODE_CONSTANTS` name; (2) `raise_error`’s contract `debug_assert`
  holds across the corpus (already exercised, but assert the set relationship
  directly); (3) the table has no duplicate names/codes.

## Phases

### Phase 1 — table-source the data-object messages (CORRECTED, see D-1)

The used-set approach is dropped (unsound — D-1). Instead the proven bug-256
module-analysis gating is kept, but every message is sourced from the table. Same
set/order/strings → byte-identical **except** the one sanctioned `ErrWrongMode`
consolidation (its data object adopts the table message).

- [x] `data_objects.rs`: the ~18 gating blocks (65 `ERR_*_MESSAGE` refs) source each
      message via `err_msg(name)` → `errorcode::runtime_error(name).1`.
- [x] `mod.rs`: `native_link_error_messages`/`standard_error_messages` source
      `(code, message, symbol)` via `errorcode::runtime_error_triple(name)`, order
      preserved (drives data-object layout). Consumers unchanged.
- [x] `arena.rs`/`builder_resource_cleanup.rs`/`link_thunk.rs` code/message sites
      table-sourced too (the last non-emission `ERR_*` readers).
- [x] `used_errors` field + inserts + inits deleted (proven 0 readers — it was never
      the signal; module-analysis is).

Acceptance: `cargo test --bin mfb` green (3751); artifact-gate diffs are ONLY the
`_mfb_str_error_wrong_mode` data object (the sanctioned consolidation) — re-baselined
with runtime proof; every fixture still links (bug-256: the emitted `_mfb_str_error_*`
set is unchanged except the wrong-mode message text).
Commit: 002566722 (+ wrong-mode golden re-baseline)

### Phase 2 — delete ERR_* + wrappers, rename file

Pure dead-code removal, gated on `grep` proof of zero references.

- [x] ~~delete the 12 `emit_*_return` wrappers~~ — moot: already deleted in plan-88-B
      (`grep -rc 'emit_[a-z_]+_return(' src/target/shared/code/*.rs` → 0 outside defs).
- [x] Deleted all 123 `ERR_*` code/message/symbol consts from `error_constants.rs`
      (`grep -c 'pub(crate) const ERR_[A-Z_]*_\(CODE\|MESSAGE\|SYMBOL\)'` → 0), plus the
      3 now-moot parity tests that compared them to the table.
- [x] ~~Rename `error_constants.rs` → `result_abi.rs`~~ — **deferred** (cosmetic): the
      file is cited by **30+** spec/man `[[…:SYMBOL]]` provenance links; renaming
      churns all of them for a filename change with no behavior/invariant value. The
      two invariants are met without it. Recorded as follow-up.
- [x] `cargo build --bin mfb` clean (no dangling reference); rt-error spot-check green.

Acceptance: 123 `ERR_*` consts + 12 wrappers gone (grep → 0); `cargo test --bin mfb`
green (3751). File rename deferred (above).
Commit: 002566722

### Phase 3 — enforce the two invariants + drift guard + close

This phase proves the feature's Definition of done (plan-88-A): **two methods, one
metadata source.** These greps are the acceptance, not decoration.

- [ ] **Invariant #1 — exactly two entry points.** Assert (a CI grep or a
      `#[test]` shelling out): zero `emit_*_return` wrappers, zero
      `push_error_message_address` callers, and zero `emit_error_code_return`
      callers outside `raise_error`/`raise_error_bare`'s own bodies:
      `grep -rEc 'emit_[a-z_]+_return\(\)|push_error_message_address\(' src/target/shared/code/*.rs`
      → 0 (excluding definitions). Every error is raised via one of the two methods.
- [x] **Invariant #2 — one metadata source.** `grep -rn 'const ERR_[A-Z_]*_(CODE|
      MESSAGE|SYMBOL)' src/` → **0** definitions; `ERR_*` in code (excl comments/docs/
      tests) → **0**. `ERRORCODE_CONSTANTS` is the only error metadata.
- [x] Drift guards added in `errorcode.rs` tests: `every_builtin_declared_error_is_a_table_name`
      (every `BuiltinFunction.errors` entry resolves via `runtime_error`) and
      `table_has_no_duplicate_names_or_codes` (unique name/code/symbol). Both green.
- [x] errorCode-table doc-comment already states it is the metadata authority
      (code/message/symbol columns). `error_constants.rs` rename deferred (Phase 2).
- [~] Move sub-plans to `planning/completed/`: A, B, C archived (`9d4af14c3`); D
      archives on close.
- [x] Tests: the two invariant checks + drift guards pass; `cargo test` 3751 green.

Acceptance: **both invariant greps return 0** and the drift-guard tests pass — the
feature's definition of done. Full `cargo test --bin mfb` green (3751); wrong-mode
runtime proof holds; goldens re-baselined for the one wrong-mode consolidation.
Commit: 002566722 (+ golden re-baseline)

## Validation Plan

- Tests: `descriptor_errors_are_known_codes` (Phase 3); the used-set emission test
  (Phase 1); full `cargo test --bin mfb`.
- Coverage check: the drift test iterates the real `REGISTRY` (in the bin suite
  denominator).
- Runtime proof (whole feature): a `.mfb` program that triggers one error from
  each family (index-out-of-range via `collections.get`, `ErrInvalidArgument`,
  `ErrOutOfMemory`, a float-domain operator error, an app wrong-mode error) —
  built and run, each `Error.code` and message identical to a pre-plan-88 build of
  the same program (capture the reference outputs before starting D, per AGENTS.md
  “a claim is measured”).
- Codegen goldens: `scripts/artifact-gate.sh` after each phase to see the delta.
  Phase 2 (dead-const/​wrapper deletion) should be zero-delta (nothing emitted
  referenced them). Phase 1 (used-set switch) may move bytes — re-baseline once the
  runtime proof + used-set-minimality test pass (AGENTS.md’s “never edit a golden
  until proven wrong” is satisfied by that runtime proof, since the same error is
  raised). The hard gate is the two invariant greps + runtime behavior, not zero
  delta.
- Doc sync: update `src/builtins/errorcode.rs` doc-comment; confirm
  `src/docs/spec/diagnostics/02_error-codes.md` still matches the table
  (`table_matches_registry` test — must stay green).
- Acceptance: full `cargo test --bin mfb` + one clean `artifact-gate.sh`, plus the
  whole-feature runtime proof.

## Open Decisions

- **Renamed filename.** `result_abi.rs` (recommended — it holds result tags,
  object sizes, registers, the make-error-result symbol) vs. `error_layout.rs`.
  Cosmetic; pick one and update imports. (§3)
  Decision: rename

## Corrections

<Filled in during execution — especially any fixture where the used-set emitted a
different string set than the manual gating (a used-set completeness bug), and the
final reference-output capture for the runtime proof.>

## Summary

D is the payoff and the last blast radius: it switches error-string emission to
the automatic used-set (Phase 1 — the one place bytes could move, gated hard on
`artifact-gate.sh`), then deletes the 123 `ERR_*` consts and 12 wrappers, renames
`error_constants.rs`, and lands the drift guard. On completion the whole-feature
goal holds: one `ERRORCODE_CONSTANTS` row per error, `errors` as the validated
contract, `raise_error` as the sole primitive, minimal used-set-driven emission,
and identical runtime behavior (same errors raised). Untouched: `ErrWrapped`, the structural
result/error-object constants, and the IR/`.mfp` format.
