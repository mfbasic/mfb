# plan-107-B: Relocate the general semantic cluster into ir::verify

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-107-A (the verdict table and the set-equality harness exist;
the recipe is priced).

Relocate every (V)-verdict rule from A's audit that is **not** in the
`NATIVE_*`/LINK or `TESTING_*` families — the general semantic cluster
(expected from A's hypothesis: `TYPE_UNKNOWN_VALUE`, `TYPE_DUPLICATE_FIELD`,
`TYPE_RECOVER_TYPE_MISMATCH`, `TYPE_THREAD_NOT_SENDABLE`,
`TYPE_ISOLATED_NOT_VISIBLE`, `TYPE_RESULT_NOT_USER_VISIBLE`,
`TYPE_COLLECTION_OWNERSHIP_VIOLATION`, `RESOURCE_SHADOWS_BUILTIN`,
`MONEY_INEXACT_FLOAT_LITERAL` if (V), plus whatever A reclassifies) — one rule
per commit, through the A recipe.

See plan-107-A for the shared prerequisites, the gate policy (set equality +
deliberate golden re-pins + byte-identical codegen), and the recipe.

## Prerequisites

See plan-107-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-107-A complete | verdict table has no hypothesis rows; 3 pilots landed | NOT MET until A lands |

## 1. Goal

- Every general-cluster (V) rule is implemented in `ir::verify` (typed IR
  reads, IR-span locations), listed in `RELOCATED_TO_IR_VERIFY`, and its
  syntaxcheck implementation is **deleted** — per rule, per commit.
- Each relocated rule fires on the package path where its shape is reachable
  in decoded IR (fixture per rule, per A's coverage requirement).
- The corpus is diagnostic-set-equal throughout; every re-pinned golden is
  listed in its relocation commit.

### Non-goals (explicit constraints)

- Codegen byte-identical throughout (`artifact-gate all` per commit batch).
- No wording/message changes; no rule semantics changes; inference-fact ports
  into verify reproduce syntaxcheck's derivation exactly (A's audit lists
  each needed fact).
- (S)- and (I)-verdict codes untouched (D's scope).

## 2. Current State

Set by A's verdict table — this letter's task list IS that table's general
cluster. Re-scope here in place if A's verdicts move rules between B and C
(record the delta in Corrections).

### Measured populations

| What | Count | Command |
|---|---|---|
| general-cluster (V) rules | from A's table (hypothesis: ~10) | plan-107-A §2 verdicts |
| corpus fixtures per rule | from A's audit | recorded per rule in A |
| verify inference-fact gaps to port | from A's audit | recorded per rule in A |

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

- [ ] Relocate each general-cluster rule with no inference gap (list from A's
      table — enumerate them here at kickoff as checkboxes, one per rule).
- [ ] Tests: corpus + harness per commit; package-path fixture per rule.

Acceptance: those rules live only in verify; corpus set-equal; gate
byte-identical; goldens re-pinned and listed.
Commit: — (one hash per rule, recorded against its checkbox)

### Phase 2 — gap-bearing rules

- [ ] Port each needed inference fact into verify (unit-tested), then relocate
      its rule(s) (enumerate from A's table at kickoff).
- [ ] Tests: as Phase 1 + the inference-fact units.

Acceptance: the general cluster fully relocated; syntaxcheck's copies deleted;
corpus set-equal; gate byte-identical.
Commit: — (per rule)

## Validation Plan

- Tests: harness per commit; package-path fixtures; inference units; full
  suite at letter end.
- Coverage check: A's per-rule fixture counts — no rule moves unguarded.
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: none (D owns it).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- None — decisions live in A's verdicts; a new fork discovered mid-cluster is
  recorded here with a recommendation before proceeding.

## Corrections

<Filled in during execution.>

## Summary

Pure production-line work behind the harness. Risk is per-rule wording/span
fidelity, held by one-rule-per-commit attribution; every move upgrades the
package path's guard for free.
