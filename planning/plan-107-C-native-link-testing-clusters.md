# plan-107-C: Relocate the NATIVE/LINK cluster (+ TESTING family per verdict)

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-107-B (the general cluster is done; the recipe and harness
are battle-tested on ~13 more rules).

Relocate the `NATIVE_*` LINK/ABI rule family (13 codes in A's census:
`NATIVE_ABI_NO_RESULT`, `NATIVE_ABI_RESULT_MARKER`, `NATIVE_ABI_UNBOUND_PARAM`,
`NATIVE_ABI_UNBOUND_SLOT`, `NATIVE_ABI_UNKNOWN_CTYPE`,
`NATIVE_BIND_IN_INVALID`, `NATIVE_CONST_OUT`, `NATIVE_CONST_UNKNOWN_SLOT`,
`NATIVE_CPTR_ESCAPE`, `NATIVE_CSTRUCT_ESCAPE`, `NATIVE_CSTRUCT_INVALID`,
`NATIVE_FREE_INVALID`, `NATIVE_STRUCT_FIELD_MISMATCH`) into `ir::verify`, and
the `TESTING_EXPECT_*` family (6 codes) **if and only if** A's audit verdicts
them (V) — the hypothesis says (S) (the `expect()` calls are desugared during
lowering), in which case they route to D's shape pass instead and this letter
records that hand-off.

See plan-107-A for shared prerequisites, gate policy, and recipe.

References:

- `src/ir/verify/link.rs` — verify's existing LINK checks (the relocation
  target module; `NATIVE_BIND_STATE_INVALID` already lives here, proving the
  family's facts are reachable from IR).
- `src/syntaxcheck/link.rs` — the source implementations moving out.
- `NirModule`/`IrProject` LINK carriage — `IrLinkFunction`/`IrCStruct`/
  `IrNativeResource` ride to IR **verbatim** (plan-50/53/59 history), which is
  why this family is expected fully (V).

## Prerequisites

See plan-107-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-107-B complete | general cluster relocated; B's boxes ticked | NOT MET until B lands |

## 1. Goal

- All 13 `NATIVE_*` rules implemented in `verify/link.rs` (typed IR LINK
  tables), listed, syntaxcheck's `link.rs` implementations deleted.
- **Package-path coverage is mandatory here, not best-effort**: LINK
  declarations ship inside packages, so every one of these rules gets a
  crafted-`.mfp`/package fixture proving it fires on decoded IR — this family
  is precisely where the "syntaxcheck-only rule doesn't guard packages" gap
  bites hardest (a hostile package's LINK/ABI declarations reach codegen's
  thunk emitter).
- The `TESTING_EXPECT_*` verdict from A is executed: relocated here if (V),
  or formally handed to D's shape pass if (S) — either way resolved, not
  dangling.

### Non-goals (explicit constraints)

- Per plan-107-A (set equality, byte-identical codegen, no wording changes).
- No changes to LINK lowering/thunk emission — only where the *checks* live.

## 2. Current State

Split rule family: `verify/link.rs` already owns the BIND STATE validation
(sole implementer since plan-53-B); syntaxcheck's `link.rs` owns the 13 above.
The facts they read (ctypes, slots, buffer markers, CSTRUCT fields) are all
present in the IR LINK tables (carried verbatim).

### Measured populations

| What | Count | Command |
|---|---|---|
| `NATIVE_*` codes to move | 13 | plan-107-A §2 census |
| `TESTING_EXPECT_*` codes pending verdict | 6 | plan-107-A §2 census |
| syntaxcheck/link.rs size | at kickoff | `wc -l src/syntaxcheck/link.rs` |
| corpus fixtures per rule | from A's audit | recorded in A |

### Verified properties

- **LINK data rides to IR verbatim** — `IrLinkFunction`/`IrCStruct`/
  `IrNativeResource` carried unchanged from source to IR (read
  `src/ir/lower_link.rs`; module docs). So the family's evidence survives
  lowering. VERIFIED (read during plan-102/`6db8e040b` work).
- Whether every `NATIVE_*` rule's *specific* fact (e.g. escape analysis for
  `NATIVE_CPTR_ESCAPE`) is derivable from IR alone: UNVERIFIED per-rule — A's
  audit rows govern; any rule whose fact is source-shaped goes to D's residue
  with the evidence recorded.

## 3. Design Overview

Same production line as B, into `verify/link.rs`. The package fixtures are
built per rule BEFORE its relocation (RED on the pre-move tree where the
package path is unguarded, GREEN after — the security gap made visible, then
closed).

### Rejected alternatives

- **Leave the NATIVE family in syntaxcheck since LINK programs are rare.**
  Rejected: rarity is not a guard; the package path is exactly where hostile
  input arrives, and this family configures native thunks.

## Compatibility / Format Impact

Per plan-107-A.

## Phases

### Phase 1 — package-path fixtures (the gap made visible)

- [ ] For each of the 13 rules: a package fixture whose decoded IR violates
      the rule; record which currently pass UNGUARDED through the package path
      (expected: all whose shapes a `.mfp` can carry).
- [ ] Tests: fixtures committed RED-annotated (guarded only on the source
      path) — the harness records the pre-state.

Acceptance: per-rule package fixtures exist with recorded pre-state.
Commit: —

### Phase 2 — relocate the family

- [ ] One rule per commit into `verify/link.rs` (recipe); Phase-1 fixtures
      flip to firing on the package path; syntaxcheck `link.rs` copies
      deleted.
- [ ] Execute the TESTING-family verdict (relocate here, or record the formal
      hand-off to D).
- [ ] Tests: corpus + harness per commit; the package fixtures.

Acceptance: 13 rules verify-only and package-guarded; TESTING verdict
executed; corpus set-equal; gate byte-identical.
Commit: — (per rule)

## Validation Plan

- Tests: package fixtures per rule; corpus + harness; full suite.
- Coverage check: A's per-rule fixture counts + the new package fixtures.
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: none (D owns it); note `.ai/resources-packages.md` may describe
  where LINK validation lives — flag for D's pass.
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- None pending beyond A's TESTING verdict, which this letter executes.

## Corrections

<Filled in during execution.>

## Summary

The security-payoff letter: the family most exposed to hostile packages moves
onto the path that actually guards them, with the gap demonstrated by fixture
before it is closed. TESTING's fate is decided by A's evidence, not by
assumption.
