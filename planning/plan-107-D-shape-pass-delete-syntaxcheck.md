# plan-107-D: hir::shape for the erased-evidence residue; DELETE src/syntaxcheck

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-107-C (every (V) rule lives in ir::verify; only the (S)
residue and the shell remain in syntaxcheck).

Move the erased-evidence residue — the rules whose constructs total lowering
destroys (per A's verdicts; hypothesis: the named-argument pair, the
`EXIT_*`/`SUB_RETURN` flavors, the inline-TRAP/RECOVER family,
`UNREACHABLE_AFTER_EXIT`, `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`, and the
`TESTING_EXPECT_*` family if C handed it off) — into a new, small
**`src/hir/shape.rs`** pass over the HIR, then **delete `src/syntaxcheck/`
entirely** and retire the dual-checker machinery: `RELOCATED_TO_IR_VERIFY`,
`syntaxcheck::report`'s skip logic, and the two-stream concatenation. The end
state is the one `rules/mod.rs` names: **`ir::verify` is the single semantic
checker**, plus one explicitly-scoped pre-lowering shape pass whose every rule
carries a one-line justification of what lowering erases.

See plan-107-A for shared prerequisites, gate policy, and the census.

References:

- `src/cli/build/mod.rs:359-370` — the split comment this letter deletes
  (replaced by the shape-pass rationale).
- `src/rules/mod.rs:17-22` — the merge contract; post-D the render is
  shape-stream + verify-stream (two streams still, but one is ~5% the size
  and named for what it is).
- `planning/Compiler Pipeline.md:47-48` — Rec #3's `debug_assert!`-guarded
  dual-list hazard: dies with the list.
- `src/audit/mod.rs` — the audit path runs the same validators; switches with
  the build path.

## Prerequisites

See plan-107-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-107-C complete | every (V) rule verify-only; C's boxes ticked | NOT MET until C lands |

## 1. Goal

- `src/hir/shape.rs` exists: one walk over `&HirProject` implementing exactly
  the (S)-verdict rules, each with a doc comment naming the erased evidence
  ("lowering normalizes named arguments to positional — this fact does not
  exist in IR"). Collected diagnostics, merged before verify's stream
  (preserving each rule's current stream position — they render from the
  first stream today, so set AND order stay stable for these).
- **`rg -n 'syntaxcheck' src/` → 0** (directory deleted; call sites gone;
  `check_project_collect`, `export_in_executable_diagnostics`, and every
  module under `src/syntaxcheck/` removed — `export_in_executable` relocated
  per its (I)/(V) verdict from A).
- `RELOCATED_TO_IR_VERIFY` and the skip logic **deleted** (with no second
  emitter, the sole-rejecter list is meaningless — and Rec #3's silent-release
  hazard is gone structurally, which beats hardening it).
- Both seams (build + audit) run: `hir::shape` on the concrete HIR +
  `ir::verify` on the IR. Nothing else checks anything.

### Non-goals (explicit constraints)

- Per plan-107-A: codegen byte-identical; diagnostic sets unchanged; order
  changes only where a rule's stream membership changed in B/C (already
  re-pinned there) — D itself should be order-neutral (the shape rules keep
  first-stream position; verify's set is untouched).
- The dual-run topology (a pre-lowering pass + verify both running to
  completion) is KEPT — it is now the honest minimum, not debt.
- Resolver/monomorph short-circuit behavior unchanged (separate work).

## 2. Current State (entering D)

Post-C, `src/syntaxcheck/` contains: the (S) rules, the walk driving them,
whatever shared helpers they still use, and dead weight from the departed
rules. Its size at D's kickoff is the first measurement.

### Measured populations

| What | Count | Command |
|---|---|---|
| (S) rules to move | from A's verdicts (hypothesis ~15–20 incl. TESTING) | plan-107-A §2 |
| syntaxcheck residual size at kickoff | measure | `find src/syntaxcheck -name '*.rs' \| xargs wc -l` |
| syntaxcheck call sites to remove | measure at kickoff | `rg -n 'syntaxcheck' src/ --type rust -l` |
| RELOCATED list + skip-logic sites | 74-entry list + `syntaxcheck::report` skip + `rules/mod.rs` references | `rg -n 'RELOCATED_TO_IR_VERIFY' src/` |

### Verified properties

- **The (S) rules' evidence exists in HIR** — HIR mirrors the AST 1:1
  (named `HirCallArg::Named`, `ExitTarget` flavors, `HirExpression::Trapped`,
  lambdas), so a shape pass over concrete HIR sees everything syntaxcheck saw.
  VERIFIED by the HIR data model (`src/hir/mod.rs`) + the four landed
  AST→HIR ports.
- Whether any (S) rule needs *pre-monomorph* shapes (e.g. named-argument
  checks against the original overload's parameter names before mangling):
  UNVERIFIED — A's audit rows carry the answer; a pre-monomorph rule runs the
  shape pass on the generic HIR instead (both are in hand on the build path;
  the pass takes whichever its rules need, decided per rule with evidence).

## 3. Design Overview

Port the (S) rules as one small typed walk (the recipe from `6db8e040b`'s
ports: mirrored variants, structural type reads), wire it at both seams, then
delete: `src/syntaxcheck/` wholesale, its call sites, the list, the skip
logic, and the split comment — replaced by the shape-pass rationale. Then the
docs pass.

**Risk concentration:** deletion fallout — helpers syntaxcheck still exports
that something else quietly uses. Mitigation: `rg -n 'syntaxcheck' src/` → 0
is the acceptance, and the compiler enforces it; anything that breaks is a
dependency to relocate explicitly, not silently.

### Rejected alternatives

- **Keep a gutted syntaxcheck as the shape pass's home.** Rejected: the name
  is the misdirection ("syntaxcheck" doing semantics confused this codebase's
  own review); a fresh, small, correctly-named module with per-rule
  justifications is the maintainable end state.
- **Fold the shape rules into `elaborate`.** Rejected (recorded in A):
  validation coupled into construction makes both harder to reason about.

## Compatibility / Format Impact

None to codegen/wire. Diagnostic sets unchanged; order stable in D itself.

## Phases

### Phase 1 — hir::shape + the (S) ports

- [ ] Create `src/hir/shape.rs`; port each (S) rule (one commit per rule,
      corpus + harness each) with its erased-evidence doc line; wire into the
      build + audit seams in the first-stream position.
- [ ] Tests: corpus set-equal AND order-equal (D is order-neutral); full
      suite.

Acceptance: every (S) rule fires from `hir::shape`; syntaxcheck's copies
deleted; corpus byte-identical (set and order).
Commit: — (per rule)

### Phase 2 — delete src/syntaxcheck + the split machinery

- [ ] Delete `src/syntaxcheck/` and all call sites; relocate any straggler
      helper some other module used (explicitly, with a note here).
- [ ] Delete `RELOCATED_TO_IR_VERIFY`, the `syntaxcheck::report` skip logic,
      the `rules/mod.rs` split references; replace `build/mod.rs:359`'s split
      comment with the shape-pass rationale.
- [ ] Tests: full suite; the whole `*-invalid` corpus; `artifact-gate all`.

Acceptance: `rg -n 'syntaxcheck\|RELOCATED_TO_IR_VERIFY' src/` → 0; corpus
set-equal; gate byte-identical; full suite green.
Commit: —

### Phase 3 — docs pass + closing census

- [ ] `.ai/compiler.md` (checking topology: shape + verify),
      `.ai/testing-gates.md` (check-pass topology), `.ai/resources-packages.md`
      (LINK validation home), spec `02_frontend.md` — the review's
      "syntaxcheck" descriptions replaced.
- [ ] Closing census recorded here: rule count in `hir::shape` (each with its
      justification line), rule count in verify, zero elsewhere; the
      plan-20-Z era formally closed.

Acceptance: docs updated; census recorded; full suite; gate; test-accept; fmt
both crates.
Commit: —

## Validation Plan

- Tests: per-rule corpus runs; full suite; package-path fixtures from C stay
  green.
- Coverage check: A's fixture-per-rule requirement carried through the (S)
  set.
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: Phase 3.
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Generic vs concrete HIR for the shape pass** — decided per rule by A's
  audit evidence (see §2 UNVERIFIED); default concrete, promote to generic
  only where a rule's facts pre-date monomorph.

## Corrections

<Filled in during execution.>

## Summary

The deletion letter: a 14k-line misnamed subsystem is replaced by one small,
honestly-named shape pass whose every rule justifies its own existence, and
`ir::verify` becomes the single semantic checker — closing the relocation
trajectory this codebase started in plan-20 and the review's Rec #3 hazard
with it.
