# plan-106-E: Consolidation + the terminal no-strings census

Last updated: 2026-08-24
Effort: large (3h–1d) — corrected from "medium (1h–2h)" by plan-106-A Correction 2:
codegen's residual hand-rolled type-grammar sites survived plan-104 and land here
Depends on: plan-106-D (every engine typed, no backward edge; this letter
consolidates shared algebra and CERTIFIES the end state).

Finish the review's Recommendation #2 (consolidate the duplicated
type-inference/numeric-promotion walks behind single sources of truth) and
certify plan-106's terminal invariant: **no internal type-string
representation, parsing, or comparison anywhere in the compiler** — the
"NO STRINGS" end state, proven by a recorded census, not asserted.

See plan-106-A for the invariant's exact definition (the three permitted
boundary classes) and the roadmap.

References:

- `planning/Compiler Pipeline.md:28-29,68` — the sibling-walk and promotion
  censuses and the consolidation mandate.
- `src/numeric.rs` — the single typed promotion source (landed in 106-A).
- `src/codegen/engine/types/type_utils.rs` + `src/codegen/memory/…` — the
  five NIR type walks (`static_nir_value_type`, `static_type_name`,
  `static_type_name_for_fold`, `…_with_types`, `…_for_fold_with_types`),
  typed by plan-104 but still five sibling walks.
- `.ai/compiler.md`, `.ai/codegen-invariants.md`, `.ai/collections.md`,
  `src/docs/spec/architecture/02_frontend.md`/`04_ir.md`/`13_native-ir.md` —
  the docs pass.

## Prerequisites

See plan-106-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-106-D complete | `rg -n 'deelaborate' src/` → 0 | NOT MET until D lands |

## 1. Goal

- **One numeric-promotion implementation.** The measured 6 copies
  (`rg -n 'fn (numeric_binary_result_type|promote_loop_numeric_type_name)' src/`)
  are 1: `src/numeric.rs`'s typed algebra (A killed ir/lower + monomorph's 4;
  plan-104 killed codegen's; C converted syntaxcheck's; E deletes whatever
  shell remains and re-measures → exactly 1 definition, N callers).
- **The five sibling NIR walks collapse** to `static_nir_value_type` +
  environment parameters (the `_with_types`/`_for_fold` variants differ only
  in the environment consulted — the review's measured claim; verify by
  diffing the walk bodies before merging, record the diff summary here).
- **The terminal census passes and is recorded** in this file (the invariant
  from plan-106-A):
  - `rg -n 'strip_prefix("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF )' src/`
    → hits only `src/types.rs` + `src/ast/`
  - `rg -n '== "(Integer|String|Boolean|Float|Fixed|Byte|Money|Nothing|AttributeString|Scalar)"' src/`
    → 0 type-value compares (audit each residual hit; non-type string
    compares like op names are out of scope — annotate)
  - `rg -n 'format!\("(List OF|Set OF|Map OF|Result OF|MapEntry OF)' src/`
    → hits only `ParameterType::name`
  - type-valued `HashMap<String, String>` environments → 0 (per-module sweep)
  - `rg -n 'deelaborate' src/` → 0
  - `ParameterType::parse` call-site inventory → only the permitted parse-in
    boundaries (elaborate, wire decode, resolver's canonical AST-domain
    queries, tests) — list every site with its classification
- Docs and spec reflect the typed pipeline end-to-end.

### Non-goals (explicit constraints)

- No behavior change; byte-identical output and diagnostics (both corpora).
- Do not merge ENGINES across layers (lowering vs verify stay independent —
  the soundness rule requires it; consolidation means shared *algebra* and
  intra-layer sibling-walk merges only).
- No new abstraction layers (the review asks for single sources of truth, not
  a type-system framework).

## 2. Current State (at this letter's start — re-measure at kickoff)

All engines are typed (A–D, 104, 105); duplication remains as typed siblings:
the promotion shells not yet deleted, and codegen's five walks now
`ParameterType`-valued but still five bodies that must agree.

### Measured populations

| What | Count (plan-writing) | Command |
|---|---|---|
| promotion implementations | 6 → re-measure (expect 1–2 shells) | `rg -n 'fn (numeric_binary_result_type\|promote_loop_numeric_type_name)' src/` |
| sibling NIR walks | 5 | review `Compiler Pipeline.md:27`; confirm: `rg -n 'fn static_nir_value_type\|fn static_type_name' src/codegen/` |
| `.ai`/spec files needing the docs pass | 6 named in References | — |

### Verified properties

- **The `_with_types` variants "differ only in the environment they consult"**
  is the review's claim, UNVERIFIED by us — Phase 1 diffs the walk bodies and
  records the delta before any merge. If a walk embeds a genuine semantic
  difference, it is documented and kept as an explicit mode flag, not silently
  merged.

## 3. Design Overview

Small, sequential, each behind the gate: delete promotion shells → merge
sibling walks (after the body diff) → run the census → fix any straggler the
census finds (a straggler is a TASK here, never a deferral) → docs pass.

### Rejected alternatives

- **Skip the census, trust the letters.** Rejected: plan-102 shipped with
  backward seams precisely because green gates were trusted to imply
  architecture. The census IS the deliverable.

## Compatibility / Format Impact

None.

## Phases

### Phase 1 — promotion + sibling-walk consolidation

- [ ] Delete residual promotion shells; re-measure → 1 implementation.
- [ ] Diff the five NIR walk bodies; record the delta; merge onto
      `static_nir_value_type` + env params (or record the justified mode
      flags).
- [ ] Tests: the A-phase equivalence suite extended over the merged walks.

Acceptance: suite green; gate no NEW diff; walk count recorded.
Commit: —

### Phase 2 — the terminal census + straggler burn-down

- [ ] Run every census line from §1; paste the full results here.
- [ ] Any hit outside the permitted boundaries is fixed in this phase (each
      one a listed task added here as found).

Stragglers already identified by earlier letters (each a TASK here, not a
deferral — see plan-106-A §Corrections 2 and 3 for the measurements):

- [ ] **Codegen's residual hand-rolled type grammar** (plan-106-A Correction 2):
      plan-104 typed the NIR data model but left ~15 `strip_prefix("List OF "…)`
      sites and the name-keyed
      `type_utils::numeric_binary_result_type` +
      `type_utils::typed_numeric_binary_result_type` (which renders `name()`,
      runs the string algorithm, and re-matches the result). Convert onto
      `numeric::typed_binary_result_type` and structural matches.
- [ ] **Monomorph's substitution walk** (plan-106-A Correction 3):
      `concrete_type_name` (`src/monomorph/lower.rs:1630`) and its inverse
      `template_view_type` (`:1738`) parse their input, recurse by rendered
      child name, and `format!("List OF {…}")` the result — 14 type-grammar
      `format!`s, plus five `ParameterType::parse(&self.concrete_type_name(…))`
      re-parses at `lower.rs:421,475,481,862,1069`. Retype both to
      `ParameterType -> ParameterType`, preserving the two behaviors the
      by-name recursion encodes (the per-level `strip_type_group` unwrap —
      bug-105 — and the `substitutions` lookup, whose keys are always bare
      template-parameter names, so the probe belongs at `Named`/`Var` leaves).
- [ ] `bench-lowering.sh` vs the 106 baseline: record; not slower.

Acceptance: the census in this file shows the invariant HOLDS with every
residual hit classified into the three permitted boundary classes; suite
green; gate no NEW diff; `test-accept` no NEW mismatch; perf ≤ baseline.
Commit: —

### Phase 3 — docs/spec pass

- [ ] `.ai/compiler.md` (pipeline description: typed end-to-end, no
      de-elaboration), `.ai/codegen-invariants.md`, `.ai/collections.md`;
      spec `02_frontend.md`/`04_ir.md`/`13_native-ir.md` reviewed for stale
      "types are strings" claims (serialized formats unchanged — fix only
      in-memory claims).
- [ ] Memory sync: update `hir-parse-name-roundtrip-load-bearing` (the
      deelaborate dependency no longer exists) and close the loop on
      `byte-identity-cannot-see-backward-seams` (census now institutional).

Acceptance: docs updated; full suite; gate; test-accept; fmt both crates.
Commit: —

## Validation Plan

- Tests: equivalence suites; both corpora (byte-identity + diagnostics).
- Coverage check: the census sweeps `src/` wholesale — nothing outside the
  denominator.
- Runtime proof: gate; test-accept; bench vs baseline.
- Doc sync: Phase 3 IS the doc sync.
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- None — this letter executes decisions the earlier letters made. If one
  appears, it goes here with a recommendation before Phase 2 closes.

## Corrections

<Filled in during execution.>

## Summary

The certificate letter: duplication collapsed to single sources, and the
"NO STRINGS" invariant proven by recorded greps — the review's Recommendations
#1 and #2 finished, checkably, with nothing left to take on faith.
