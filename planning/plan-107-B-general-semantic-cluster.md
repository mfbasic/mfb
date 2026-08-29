# plan-107-B: Relocate the general semantic cluster into ir::verify

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-107-A (the verdict table and the set-equality harness exist;
the recipe is priced).

Relocate every **pure (V)**-verdict rule from A's audit that is **not** in the
`NATIVE_*`/LINK family, not a builtin-call typing rule (E), and not one of A's
three pilots — one rule per commit, through the A recipe. Split (V/S) rules
are NOT this letter's (they land with their shape half in D/E — see A §3).

See plan-107-A for the shared prerequisites, the gate policy (set equality +
deliberate golden re-pins + byte-identical codegen), and the recipe.

## Prerequisites

See plan-107-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-107-A complete | verdict table has no hypothesis rows; 3 pilots landed | MET 2026-08-29 — archived as `planning/completed/plan-107-A-relocation-audit-pilot.md` (`5f1f05b43`); pilots `ef53fcef3`, `2f7067fd4`, `cad6c8964` |

## 1. Goal

- Every general-cluster pure-(V) rule is implemented in `ir::verify` (typed IR
  reads, IR-span locations), listed in `RELOCATED_TO_IR_VERIFY`, and its
  syntaxcheck implementation is **deleted** — per rule, per commit.
- Each relocated rule fires on the package path (a verify unit test building
  the violating `IrProject` by hand — A's package-path precedent), per rule.
- The corpus is diagnostic-set-equal throughout; every re-pinned golden is
  listed in its relocation commit.

### Non-goals (explicit constraints)

- Codegen byte-identical throughout (`artifact-gate all` per commit batch).
- No wording/message changes; no rule semantics changes; inference-fact ports
  into verify reproduce syntaxcheck's derivation exactly (A's audit lists
  each needed fact).
- (S)-, (V/S)- and (I)-verdict codes untouched (D/E's scope).

## 2. Current State

Set by A's verdict table (plan-107-A §2, measured 2026-08-29). This letter's
population, with A's row numbers:

| Row | Code | Fixtures | verify has | Gap |
|---|---|---|---|---|
| 21 | RESOURCE_SHADOWS_BUILTIN | 0 → write one | no | none |
| 29 | TYPE_COLLECTION_OWNERSHIP_VIOLATION | 2 | Set-element + Map-key arms | port the List-element / Map-value thread arms; reproduce the double emission (declared type + literal) |
| 32 | TYPE_INLINE_TRAP_DEAD_HANDLER (Warn) | 1 | no | `$expect_` temp guard (A Open Decisions); hooks into A's `check_inline_trap_scrutinee` |
| ~~35~~ | ~~TYPE_INLINE_TRAP_REQUIRES_FALLIBLE~~ | — | — | moved to A (pilot 2, landed `2f7067fd4`) |
| 40 | TYPE_RESULT_NOT_USER_VISIBLE | 2 | no | none; measure which golden lines the resolver owns |
| 43 | TYPE_TRAP_FALLTHROUGH | 2 | handler form | port the "Normal flow reaches the TRAP" form |
| 49 | TYPE_READ_ONLY_RECORD_CONSTRUCTOR | 4 | compiler-owned form | port the `AttributedString` + `Error`/`ErrorLoc` forms with their wording |
| 37 | TYPE_LAMBDA_CAPTURE_UNSUPPORTED | 2 | no | **port `is_copyable_type`** (gap-bearing → Phase 2) |

### Measured populations

| What | Count | Command |
|---|---|---|
| general-cluster pure-(V) rules | 8 (7 gap-free + 1 gap-bearing) | plan-107-A §2 verdicts, rows above |
| corpus fixtures per rule | column above | `grep -rl --include=build.log " $CODE\]" tests` |
| verify inference-fact gaps to port | 1 (`is_copyable_type`, ~40 lines from `syntaxcheck/resources.rs:190`) | plan-107-A §2 row 37 |

## 3. Design Overview

Production-line execution of the A recipe, one rule per commit, corpus +
harness after each; rules needing an inference-fact port land the fact (with
its own unit test) in the same commit as the rule. Order within the cluster:
rules with zero inference gaps first (pure transcriptions), gap-bearing rules
last.

### Rejected alternatives

- **Batching several rules per commit.** Rejected: the harness attributes a
  set diff to a commit; batches destroy attribution.

## Compatibility / Format Impact

Per plan-107-A (order re-pins only).

## Phases

### Phase 1 — gap-free rules

- [x] RESOURCE_SHADOWS_BUILTIN — dead since plan-97 (A Corrections
      C-dead-rules): emitter, call site and predicate-free test deleted; the
      rule code retired in `rules/table.rs` (reserved, never recycled) and the
      spec (`17_native-libraries.md`, `01_rule-codes.md`) corrected; corpus
      `518 same` (0 fixtures ever tripped it)
- [x] ~~TYPE_ISOLATED_NOT_VISIBLE~~ — moot here: A's pilot 1 (`ef53fcef3`)
- [x] TYPE_RESULT_NOT_USER_VISIBLE — resolver-shadowed dead code (A
      Corrections C-dead-rules): syntaxcheck's two emitters and two tests
      deleted (`check_project_wrapper_rejects` re-based on `EXIT FUNC`); the
      resolver keeps the rule and its two goldens; corpus `518 same`
- [x] ~~TYPE_INLINE_TRAP_REQUIRES_FALLIBLE~~ — moot here: A's pilot 2 (`2f7067fd4`)
- [x] TYPE_INLINE_TRAP_DEAD_HANDLER — an arm on A's `check_inline_trap_scrutinee`
      (`inline_builtin_is_infallible(target)`); corpus `518 same, 0 reordered,
      0 set-diff` (its one fixture is warning-only); twin
      `warns_dead_handler_on_an_infallible_inline_builtin`.
- [x] TYPE_TRAP_FALLTHROUGH — second form ported (`ops[..op_index]` before
      the `Trap` op + `current_function`); corpus `517 same, 1 reordered, 0
      set-diff` after the stray-RECOVER fix (Corrections); reorder:
      `tests/syntax/json/json_read_invalid` (verify's `TYPE_FUNC_MISSING_RETURN`
      at line 5 now precedes the rule at line 9, one stream); twins
      `rejects_trap_fallthrough` (pre-existing), `rejects_normal_flow_reaching_the_trap`,
      `accepts_a_body_that_returns_before_its_trap`, `a_stray_recover_counts_as_diverging`.
- [x] TYPE_COLLECTION_OWNERSHIP_VIOLATION — List-element and Map-value
      thread arms ported (`contains_thread` + `check_collection_element_thread_free`),
      the literal sites (List/Set/Map literals) mirrored, and verify's
      `contains_resource_or_thread` fixed to walk a union's variant fields
      (it stopped at the union name — a Map key of a thread-carrying union
      passed); corpus `516 same, 2 reordered, 0 set-diff`; reorders:
      `tests/syntax/resources/bug231_resource_union_map_key_invalid`,
      `resource-collection-map-key-invalid` (the rule's two lines now follow
      verify's two `TYPE_REQUIRES_COMPARABLE` lines); twins
      `rejects_a_thread_handle_as_a_list_element`, `…_as_a_map_value`,
      `rejects_a_resource_in_a_set_literal`, `rejects_a_thread_carrying_union_as_a_map_key`.
- [x] ~~TYPE_READ_ONLY_RECORD_CONSTRUCTOR~~ — moot here: split (V/S), moved
      to D (A Corrections C-split-49)
- [ ] Tests: corpus + harness per commit; package-path (verify unit) test per
      rule.

Acceptance: those rules live only in verify; corpus set-equal; gate
byte-identical; goldens re-pinned and listed.
Commit: — (one hash per rule, recorded against its checkbox)

### Phase 2 — gap-bearing rules

- [ ] Port `is_copyable_type` into verify (unit-tested), then relocate
      TYPE_LAMBDA_CAPTURE_UNSUPPORTED (the `by_ref`/`muts` derivation per
      A §2 row 37).
- [ ] Tests: as Phase 1 + the inference-fact units.

Acceptance: the general cluster fully relocated; syntaxcheck's copies deleted;
corpus set-equal; gate byte-identical.
Commit: — (per rule)

## Validation Plan

- Tests: harness per commit; package-path tests; inference units; full
  suite at letter end.
- Coverage check: A's per-rule fixture counts — no rule moves unguarded.
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: none (D owns it).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- None — decisions live in A's verdicts; a new fork discovered mid-cluster is
  recorded here with a recommendation before proceeding.

## Corrections

- **2026-08-29 (TYPE_TRAP_FALLTHROUGH, harness SETDIFF).** Listing the rule
  exposed a fidelity gap in verify's pre-existing handler form:
  `control-flow-inline-trap-invalid` line 35 gained a second
  `TYPE_TRAP_FALLTHROUGH` ("TRAP `e` must return, fail, or propagate") on a
  handler whose only statement is a stray `RECOVER 0`. syntaxcheck's flow
  analysis treats EVERY `RECOVER` as diverging (`check_block` returns
  `AlwaysReturns` for the statement even after reporting
  `TYPE_RECOVER_OUTSIDE_INLINE_TRAP`), while lowering's fallback for a stray
  RECOVER was a discard `Eval` — erased evidence, in the fallback lowering of
  an already-erroneous statement. Fix (not a golden re-pin): the fallback now
  binds the value to a `$recover_stray` temp (`lower.rs`, `HirStatement::Recover`)
  and `block_always_returns` treats that bind as diverging — closing the same
  latent gap for the already-relocated `TYPE_FUNC_MISSING_RETURN` (a function
  ending in a stray RECOVER). Codegen-neutral by construction: a stray RECOVER
  never survives to codegen. Twin: `a_stray_recover_counts_as_diverging`.
- **2026-08-29 (from A's audit).** Scope re-set from the hypothesis list to
  A's measured verdicts: `TYPE_UNKNOWN_VALUE`, `TYPE_THREAD_NOT_SENDABLE` and
  `TYPE_ISOLATED_NOT_VISIBLE` are A's pilots; `TYPE_DUPLICATE_FIELD` and
  `TYPE_RECOVER_TYPE_MISMATCH` are split (V/S) and move to D;
  `MONEY_INEXACT_FLOAT_LITERAL` is (S) (D); `TYPE_READ_ONLY_RECORD_CONSTRUCTOR`
  (newly counted, A §Corrections C-census) joins this cluster; the hypothesis's
  "~10" is 8.

## Summary

Pure production-line work behind the harness. Risk is per-rule wording/span
fidelity, held by one-rule-per-commit attribution; every move upgrades the
package path's guard for free.
