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

### Phase 1 — activate used-set emission

Highest-risk step: the automatic emission must reproduce the manual gating’s
string *set* exactly (same errors, same messages) — bytes may shift, runtime
must not.

- [ ] Replace the 18 `push_string_value(ERR_*_MESSAGE)` gating sites in
      `data_objects.rs` with the used-set-driven emission (§3). Keep the same
      symbol derivation so bytes are identical.
- [ ] Verify on the broadest programs (a fixture using fs+net+collections+term)
      that the emitted error-string data objects match pre-D exactly.
- [ ] Tests: a test that for a module raising a known set of errors, the emitted
      data-object symbol set equals that set (no more, no fewer).

Acceptance: every fixture **links** (no dangling error-string relocation) and, run
end-to-end, raises the same error codes+messages as a pre-plan-88 build; the
used-set test proves the emitted error-string set equals the raised set (none
missing → no dangling; none extra → minimal). Goldens re-baseline for any byte
change; the gate is “links + same runtime errors + minimal set”, not zero delta.
Commit: —

### Phase 2 — delete ERR_* + wrappers, rename file

Pure dead-code removal, gated on `grep` proof of zero references.

- [ ] Confirm zero references, then delete the 12 `emit_*_return` wrappers from
      `builder_error_emission.rs` (`grep -rc 'emit_[a-z_]+_return' → 0` outside
      defs).
- [ ] Confirm zero references, then delete all 123 `ERR_*` code/message/symbol
      consts from `error_constants.rs` (`grep -rc 'ERR_[A-Z_]+_\(CODE\|MESSAGE\|SYMBOL\)' → 0`).
- [ ] Rename `error_constants.rs` → `result_abi.rs`; update `mod.rs` and all
      imports (`grep -rl 'error_constants'`).
- [ ] Tests: `cargo build --bin mfb` proves no dangling reference; the rt-error
      suite proves no behavior moved from the deletion.

Acceptance: file renamed, 123 `ERR_*` consts + 12 wrappers gone (grep → 0),
`cargo test --bin mfb` + the rt-error suite green. (Pure dead-code removal — it
should not move any golden, but the gate is the rt-error suite, not a diff.)
Commit: —

### Phase 3 — enforce the two invariants + drift guard + close

This phase proves the feature's Definition of done (plan-88-A): **two methods, one
metadata source.** These greps are the acceptance, not decoration.

- [ ] **Invariant #1 — exactly two entry points.** Assert (a CI grep or a
      `#[test]` shelling out): zero `emit_*_return` wrappers, zero
      `push_error_message_address` callers, and zero `emit_error_code_return`
      callers outside `raise_error`/`raise_error_bare`'s own bodies:
      `grep -rEc 'emit_[a-z_]+_return\(\)|push_error_message_address\(' src/target/shared/code/*.rs`
      → 0 (excluding definitions). Every error is raised via one of the two methods.
- [ ] **Invariant #2 — one metadata source.** Assert zero `ERR_*` code/message/
      symbol constants remain anywhere:
      `grep -rEc 'ERR_[A-Z_]+_(CODE|MESSAGE|SYMBOL)' src/` → 0. `ERRORCODE_CONSTANTS`
      is the only error metadata.
- [ ] Add `descriptor_errors_are_known_codes` (in `descriptor.rs` / `errorcode.rs`
      tests): every `BuiltinFunction.errors` entry across `REGISTRY` resolves via
      `errorcode::runtime_error`; the table has unique names/codes.
- [ ] Update the errorCode doc-comment(s) to note the table is the sole
      runtime-error source (code+message+symbol) and that `error_constants.rs` was
      renamed.
- [ ] Move all four plan-88 sub-plans to `planning/completed/` as they close.
- [ ] Tests: the two invariant checks + `descriptor_errors_are_known_codes` pass.

Acceptance: **both invariant greps return 0** and the drift-guard test passes —
this is the feature's definition of done. Full `cargo test --bin mfb` green; the
whole-feature runtime proof (below) holds. Goldens are re-baselined, not held at
zero delta.
Commit: —

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
