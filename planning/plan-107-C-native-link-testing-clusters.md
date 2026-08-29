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

### Phase 0 — source spans for the LINK rules (a prerequisite A did not see)

- [x] `ir::LinkSpans`: a source-path-only sidecar derived from the HIR at the
      build seam (`ir::link_spans(&HirProject)`) — per LINK function its file,
      FUNC line, ABI line, per-parameter / per-slot / per-CONST-pin lines, per
      `BIND IN` block + field lines, FREE line; per CSTRUCT its file, line and
      field lines — handed to `verify::collect_source_diagnostics` beside the
      imported-resource rows (the package path passes an empty one). No IR
      struct or wire change: the LINK tables stay as they are (their ~85
      struct-literal sites untouched) and the `.ir` dump does not print them.
- [x] verify's LINK rules attribute every emission to the span syntaxcheck
      used (slot line for slot rules, param line, `ABI` line for expression
      and buffer faults, `BIND IN` block/field lines, CSTRUCT decl/field
      lines — `check_cstruct` faults point at the named field, as syntaxcheck's
      message-match does), falling back to the function-level `<generated>`
      form on the package path exactly as today.
- [x] verify's LINK walk mirrors syntaxcheck's ORDER (per LINK block: CSTRUCT
      duplicates + layout faults, CSTRUCT escape, then per function: signature
      rules, struct-slot rules, buffer rules) — `check_link_blocks` replaces
      `check_link_functions`/`check_link_cstructs`, every rule body moved
      into per-item functions; the missing sub-forms (INOUT-non-CSTRUCT,
      IN-struct-slot RETURN, returns-struct-record, BIND IN OUT-slot /
      duplicate-field / value-shape) and the wording drifts are transcribed
      in the same pass; the unbound-IN-struct-slot gap closed.
- [x] Tests: the existing package-path twins keep their unlocated fallback
      (`ir::verify` 424 passed); the source-path attribution is proven by
      the corpus (below) rather than a hand-built HIR — the three already
      verify-only LINK rules moved from `<generated>:1` to their real lines.

Acceptance: unlisted (no source-path change), corpus `518 same`; verify's
LINK diagnostics carry source spans on the source path.
VERIFIED 2026-08-29 with one deliberate deviation: corpus `515 same, 0
reordered, 3 set-diff` — the three are the already-relocated
`NATIVE_BIND_STATE_INVALID` (`native-bind-state-invalid`,
`native-bind-state-wrong-resource`) and `TYPE_STATE_MISMATCH`
(`native-resource-state-mismatch-invalid`), whose goldens pinned the
placeholder `<generated>:1` for a SOURCE-declared LINK function; they now
report `src/lib.mfb:13`, `:28`, `:24` (re-pinned, listed in the commit).
Commit: —

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
- **2026-08-29 (C kickoff, two audit gaps).** (1) verify's LINK rules carry NO
  source location: `check_link_functions`/`check_link_cstructs` set
  `current_file = ""` and `current_line = 0` ("spans are function-level here"),
  and the IR LINK tables (`IrLinkFunction`, `IrAbiSlot`, `IrBindIn`,
  `IrCStruct`, …) have no line fields — the already-verify-only
  `NATIVE_BIND_STATE_INVALID` renders as
  `tests/syntax/native/native-bind-state-invalid/<generated>:1` in its golden.
  Every other LINK golden pins a slot/param/field line, so listing any of the
  13 without spans would SETDIFF all 22 fixtures. Phase 0 (above) adds the
  spans as a HIR-derived sidecar; the package path is unchanged. (2) A's
  string census could not see codes syntaxcheck emits DYNAMICALLY through the
  shared `ir::link` fault helpers (`self.report(fault.rule, …)`):
  `NATIVE_BUFFER_INVALID` (9 fixtures, `check_buffer_slots`) and
  `NATIVE_CSTRUCT_TOO_LARGE` (0 fixtures, `check_cstruct`) are syntaxcheck's
  on the source path and verify's on the package path, unlisted — the same
  shape as the 13. They join this letter: 15 codes, and the remaining-set
  count A recorded (49) is 51. `NATIVE_CSTRUCT_TOO_LARGE` gets its fixture
  first.
- **2026-08-29 (C kickoff, two split rules).** Reading the two implementations
  side by side: (a) `NATIVE_CONST_UNKNOWN_SLOT`'s "not a constant the compiler
  can fold" form (`syntaxcheck/link.rs`, `foldable`) is ERASED — lowering folds
  every pin to an `i64` immediate (`eval_link_const`, unfoldable → 0) and the
  pin's expression is gone; the unknown-slot form survives. (b)
  `NATIVE_FREE_INVALID`'s "malformed FREE" form checks the deallocator's
  signature (`free.param_ctype`/`return_ctype`), which `IrFree` does not carry
  (slot + symbol only — "the deallocator's signature check stays in
  syntaxcheck", `verify/link.rs`); the producer form and the empty-symbol
  sub-condition survive. Both are (V/S) and land in D with their shape halves
  (A §3 "Split rules"); C relocates the other 13. Also: verify's slot loop
  skipped CSTRUCT-typed slots entirely, so an unbound IN struct slot passed on
  the package path (syntaxcheck's second slot pass does not skip them) — fixed
  in Phase 0's mirrored walk.

## Summary

The security-payoff letter: the family most exposed to hostile packages moves
onto the path that actually guards them, with the gap demonstrated by test
before it is closed. TESTING's fate is decided by A's evidence, not by
assumption.
