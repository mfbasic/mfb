# plan-107-C: Relocate the NATIVE/LINK cluster (+ TESTING family per verdict)

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-107-B (the general cluster is done; the recipe and harness
are battle-tested on ~11 more rules).

Relocate the `NATIVE_*` LINK/ABI rule family (13 codes in A's census:
`NATIVE_ABI_NO_RESULT`, `NATIVE_ABI_RESULT_MARKER`, `NATIVE_ABI_UNBOUND_PARAM`,
`NATIVE_ABI_UNBOUND_SLOT`, `NATIVE_ABI_UNKNOWN_CTYPE`,
`NATIVE_BIND_IN_INVALID`, `NATIVE_CONST_OUT`, `NATIVE_CONST_UNKNOWN_SLOT`,
`NATIVE_CPTR_ESCAPE`, `NATIVE_CSTRUCT_ESCAPE`, `NATIVE_CSTRUCT_INVALID`,
`NATIVE_FREE_INVALID`, `NATIVE_STRUCT_FIELD_MISMATCH`) into `ir::verify`.
A's audit verdicted the `TESTING_EXPECT_*` family (6 codes) **(S)** — the
assertions are desugared by `expand_expect` during lowering and only exist
under `mfb test` — so they route to D's shape pass; this letter records that
hand-off (Phase 2) rather than relocating them.

See plan-107-A for shared prerequisites, gate policy, and recipe.

References:

- `src/ir/verify/link.rs` — verify's existing LINK checks (the relocation
  target module; `NATIVE_BIND_STATE_INVALID` already lives here, and — per
  A's audit — an implementation of EVERY one of the 13 codes already exists
  here, hand-mirrored and kept in sync by the parity test
  `native_rule_sets_agree_between_syntaxcheck_and_verify`, `verify/tests.rs:7321`).
- `src/syntaxcheck/link.rs` — the source implementations moving out.
- `NirModule`/`IrProject` LINK carriage — `IrLinkFunction`/`IrCStruct`/
  `IrNativeResource` ride to IR **verbatim** (plan-50/53/59 history), which is
  why this family is fully (V).

## Prerequisites

See plan-107-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-107-B complete | general cluster relocated; B's boxes ticked | NOT MET until B lands |

## 1. Goal

- All 13 `NATIVE_*` rules verify-only: every sub-form syntaxcheck emits is
  present in `verify/link.rs` with syntaxcheck's wording (the goldens' wording),
  listed, and syntaxcheck's `link.rs` implementations deleted.
- **Package-path coverage is mandatory here, not best-effort**: LINK
  declarations ship inside packages, so every one of these rules gets a
  verify unit test building the violating `IrProject` (the plan-58-A twin
  precedent) proving it fires on decoded IR — this family is precisely where
  the "syntaxcheck-only rule doesn't guard packages" gap bites hardest (a
  hostile package's LINK/ABI declarations reach codegen's thunk emitter).
- The `TESTING_EXPECT_*` verdict from A is executed: formally handed to D's
  shape pass (verdict (S)) — resolved, not dangling.

### Non-goals (explicit constraints)

- Per plan-107-A (set equality, byte-identical codegen, no wording changes).
- No changes to LINK lowering/thunk emission — only where the *checks* live.

## 2. Current State

Split rule family: `verify/link.rs` already owns the BIND STATE validation
(sole implementer since plan-53-B) and a package-path mirror of all 13 codes;
syntaxcheck's `link.rs` owns the source-path emission. A's fidelity diff
(plan-107-A §2 rows 7–19) found:

| Code | verify state | Work |
|---|---|---|
| NATIVE_ABI_NO_RESULT | identical | list + delete; **fixture first** (0 corpus fixtures) |
| NATIVE_ABI_RESULT_MARKER | 1 of 2 forms | port the struct-slot-is-IN form (`syntaxcheck/link.rs:243`) |
| NATIVE_ABI_UNBOUND_PARAM | identical | list + delete; **fixture first** (0) |
| NATIVE_ABI_UNBOUND_SLOT | wording drift (`SUCCESS_ON/RESULT` vs golden's `SUCCESS_ON/RETURN`) | fix wording |
| NATIVE_ABI_UNKNOWN_CTYPE | 2 of 3 forms | port the INOUT-non-CSTRUCT form (`link.rs:199`) |
| NATIVE_BIND_IN_INVALID | 5 of 6 forms, 3 wordings differ | port + re-word (`link.rs:283,319,345`) |
| NATIVE_CONST_OUT | identical | list + delete |
| NATIVE_CONST_UNKNOWN_SLOT | 1 of 2 forms | port the unfoldable-pin form (`link.rs:693`) |
| NATIVE_CPTR_ESCAPE | identical | list + delete |
| NATIVE_CSTRUCT_ESCAPE | wording drift | fix wording |
| NATIVE_CSTRUCT_INVALID | identical | list + delete |
| NATIVE_FREE_INVALID | malformed-FREE wording drift | fix wording |
| NATIVE_STRUCT_FIELD_MISMATCH | 1 of 2 forms | port the returns-struct-slot form (`link.rs:254`) |

Also verify's diagnostic LINE for each form must match syntaxcheck's (which
reports at the `ABI` line for expression-derived facts — `link_expr_idents`
comment, `syntaxcheck/mod.rs:171`); measured per rule by the harness.

### Measured populations

| What | Count | Command |
|---|---|---|
| `NATIVE_*` codes to move | 13 | plan-107-A §2 census |
| `TESTING_EXPECT_*` codes handed to D | 6 | plan-107-A §2 rows 23–28 |
| syntaxcheck/link.rs size | 1937 lines | `wc -l src/syntaxcheck/link.rs` (2026-08-29) |
| corpus fixtures per rule | 0,1,0,6,4,1,1,1,1,1,2,2,1 (table order) | plan-107-A §2 |

### Verified properties

- **LINK data rides to IR verbatim** — `IrLinkFunction`/`IrCStruct`/
  `IrNativeResource` carried unchanged from source to IR (read
  `src/ir/lower_link.rs`; module docs). So the family's evidence survives
  lowering. VERIFIED (read during plan-102/`6db8e040b` work; re-confirmed by
  A: verify already evaluates every one of the 13 rules on decoded IR).
- Every `NATIVE_*` rule's *specific* fact is derivable from IR alone —
  VERIFIED by the existing verify mirror (A §2 rows 7–19); the remaining work
  is sub-forms and wording, not derivability.

## 3. Design Overview

Same production line as B, into `verify/link.rs`. The package-path unit
tests are written per rule BEFORE its relocation (RED on the pre-move tree
where a sub-form is missing, GREEN after — the security gap made visible,
then closed); for the sub-forms verify already has, the twin exists or is
added for completeness.

### Rejected alternatives

- **Leave the NATIVE family in syntaxcheck since LINK programs are rare.**
  Rejected: rarity is not a guard; the package path is exactly where hostile
  input arrives, and this family configures native thunks.

## Compatibility / Format Impact

Per plan-107-A.

## Phases

### Phase 1 — package-path tests (the gap made visible)

- [ ] For each of the 13 rules: a verify unit test per sub-form whose IR
      violates the rule; record which sub-forms currently pass UNGUARDED
      through the package path (expected: the 5 missing forms in §2).
- [ ] Source fixtures for the two zero-fixture rules
      (`NATIVE_ABI_NO_RESULT`, `NATIVE_ABI_UNBOUND_PARAM`).

Acceptance: per-rule package tests exist with recorded pre-state.
Commit: —

### Phase 2 — relocate the family

- [ ] One rule per commit into `verify/link.rs` (recipe); Phase-1 tests
      flip to firing on the package path; syntaxcheck `link.rs` copies
      deleted.
- [ ] Execute the TESTING-family verdict: record the formal hand-off to D
      here (rows 23–28 are (S); D's Phase 1 lists them).
- [ ] Tests: corpus + harness per commit; the package tests; the parity test
      is retired in the final commit (with syntaxcheck's `link.rs` gone there
      is no second set to agree with).

Acceptance: 13 rules verify-only and package-guarded; TESTING verdict
executed; corpus set-equal; gate byte-identical.
Commit: — (per rule)

## Validation Plan

- Tests: package tests per rule; corpus + harness; full suite.
- Coverage check: A's per-rule fixture counts + the new package tests.
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: none (D owns it); note `.ai/resources-packages.md` may describe
  where LINK validation lives — flag for D's pass.
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- None pending beyond A's TESTING verdict, which this letter executes.

## Corrections

- **2026-08-29 (from A's audit).** Verify already mirrors all 13 codes; the
  work is 5 missing sub-forms + 5 wording drifts (§2 table), not 13 ports.
  The TESTING family is (S): handed to D.

## Summary

The security-payoff letter: the family most exposed to hostile packages moves
onto the path that actually guards them, with the gap demonstrated by test
before it is closed. TESTING's fate is decided by A's evidence, not by
assumption.
