# plan-106-D: Validators consume HIR; delete de-elaboration entirely

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-106-C (syntaxcheck speaks ParameterType; only its INPUT is
still the rendered AST).

Switch the post-monomorph validators — `syntaxcheck::check_project_collect`,
`resolver::resolve_augmented`, and `manifest::entry::validate_entry_point` —
from the de-elaborated AST to the **concrete HIR**, on both the build path
(`cli/build/mod.rs:341`) and the audit path (`audit/mod.rs:111`). Then delete
the entire de-elaboration machinery (16 `deelaborate_*` functions in
`src/hir/mod.rs`) and the test-inspection uses. After this letter **no HIR→AST
conversion exists anywhere** — the last backward edge in the compiler is gone.

See plan-106-A for the roadmap, shared prerequisites, and the terminal
invariant.

References:

- `src/hir/mod.rs` — the de-elaboration block (its own comment already names
  this letter's condition: "retired when those validators move onto HIR").
- `src/cli/build/mod.rs:341-400`, `src/audit/mod.rs:108-125` — the two seams.
- `src/syntaxcheck/` — post-C, `ParameterType`-typed but AST-walking.
- `src/resolver/mod.rs:68-99` — `resolve_augmented` and the `validate_docs`
  bool threading (the review's dual-resolve observation, `Compiler
  Pipeline.md:40`).
- `src/manifest/entry.rs` — `validate_entry_point(… &AstProject)`.
- `planning/completed/plan-102-D-elaborate-generics.md` §Corrections — where
  this seam was recorded as deliberate debt.

## Prerequisites

See plan-106-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-106-C complete | `rg -n 'enum Type' src/syntaxcheck/` → 0 | NOT MET until C lands |

## 1. Goal

- `rg -n 'deelaborate' src/` → **0 hits** (functions deleted, both production
  seams and every `#[cfg(test)]` inspection helper gone — tests assert over
  HIR directly).
- `check_project_collect`, `resolve_augmented`, and `validate_entry_point`
  take `&HirProject` (HIR mirrors the AST 1:1 — the walks port mechanically,
  as `resource_escape`/`expand_expect` did in commit `6db8e040b`; type facts
  are read structurally, no `.name()` re-derivation).
- Build and audit paths pass `&concrete_hir` directly; the render, its clone
  cost, and the `parse↔name` dependency at this seam all disappear.

### Non-goals (explicit constraints)

- No behavior change: same diagnostics (codes/wording/order — the full
  corpus), same accept/reject set, same entry-validation errors.
- No rule relocation and no dual-resolve restructuring (the review's Rec #5/#6
  observations about resolve-runs-twice and diagnostic streaming are REAL but
  they are separate work — record them, don't braid them).
- The PRE-monomorph passes (`resolve_project` on the source AST, DOC
  validation) stay AST-domain — they run before elaboration by design.

## 2. Current State

Post-`6db8e040b` the compile path is forward-only and exactly one production
render remains: `deelaborate(&concrete_hir)` feeding the three validators at
the two seams. The de-elaboration block is 16 private functions behind one
`pub(crate)` entry; monomorph/ir tests also use it for result inspection.

### Measured populations

| What | Count | Command |
|---|---|---|
| `deelaborate_*` functions to delete | 16 | `rg -n 'fn deelaborate' src/hir/mod.rs \| wc -l` → 16 |
| production seams | 2 | `cli/build/mod.rs:341`, `audit/mod.rs:111` |
| test-inspection call sites to port | 5 | `rg -n 'hir::deelaborate' src/monomorph/lower.rs src/ir/tests.rs \| wc -l` → 5 (all `#[cfg(test)]`) |
| validator entry points to retarget | 3 | `check_project_collect`, `resolve_augmented`, `validate_entry_point` |
| syntaxcheck walk surface (AST→HIR port) | 14,441 lines total; walk-arm count at kickoff | `rg -c 'Statement::\|Expression::\|Item::' src/syntaxcheck/` (record at kickoff — post-C the count reflects the real port surface) |
| resolve_augmented walk surface | at kickoff | `rg -c 'Statement::\|Expression::\|Item::' src/resolver/` |

### Verified properties

- **HIR mirrors the AST 1:1** with identical variant names — the port recipe
  (perl word-boundary renames + type-field reads becoming structural) is
  proven four times over (monomorph D3, `resource_escape`, `expand_expect`,
  ir::lower C3). VERIFIED (landed).
- **What `resolve_augmented` does post-monomorph** (vs the pre-monomorph
  `resolve_project`): UNVERIFIED in detail — Phase 1 reads it and records
  which of its checks are name-resolution (port) vs type-string work
  (should already be gone post-105-B) vs redundant with the pre-pass
  (candidate to note for the separate dual-resolve cleanup, NOT to delete
  here).

## 3. Design Overview

Port order = smallest first, corpus after each: `validate_entry_point`
(smallest), `resolve_augmented`, then syntaxcheck's walk (largest surface but
mechanical post-C — its type logic is already `ParameterType`; only the node
types change). Then flip the two seams, delete the block, port the test
inspections to HIR assertions.

**Correctness risk:** syntaxcheck's walk breadth — thousands of match arms
across 14k lines. Mitigation is the proven recipe plus committing
module-by-module with the full diagnostic corpus after each.

### Rejected alternatives

- **Relocate all rules into ir::verify instead of porting the walk.**
  Rejected for this plan: that is the years-long rule-by-rule reproduction
  trajectory already underway in this codebase (each rule individually
  golden-verified); the port gets to no-strings/no-backward NOW without
  changing the two-pass topology. Rule relocation can continue afterwards
  independently.

## Compatibility / Format Impact

None. Diagnostics byte-identical.

## Phases

### Phase 1 — entry validation + resolve_augmented onto HIR

- [ ] Read `resolve_augmented` post-monomorph responsibilities; record the
      inventory in §2 (replacing UNVERIFIED).
- [ ] Port `validate_entry_point` to `&HirProject` (both callers).
- [ ] Port `resolve_augmented` to `&HirProject`.
- [ ] Tests: entry-validation unit tests; resolver corpus.

Acceptance: suite green; diagnostic corpus byte-identical; gate no NEW diff.
Commit: —

### Phase 2 — syntaxcheck walk onto HIR

- [ ] Port `check_project_collect`'s walk module-by-module
      (`Statement::`→`HirStatement::` etc.; type fields read structurally),
      full diagnostic corpus after each module.
- [ ] Tests: full `*-invalid` corpus; accepted-program gate.

Acceptance: suite green; corpus byte-identical; gate no NEW diff.
Commit: —

### Phase 3 — flip the seams; delete de-elaboration

- [ ] `cli/build/mod.rs` + `audit/mod.rs` pass `&concrete_hir`; delete the
      renders.
- [ ] Delete all 16 `deelaborate_*` fns + the block comment; port the 5
      test-inspection sites to assert over HIR.
- [ ] Tests: full suite.

Acceptance: `rg -n 'deelaborate' src/` → **0**; suite green; gate no NEW diff;
`test-accept` no NEW mismatch.
Commit: —

## Validation Plan

- Tests: per-module diagnostic corpus; entry/resolver units; full suite.
- Coverage check: the corpus exercises all 124 rules (measured).
- Runtime proof: gate byte-identical; `test-accept`.
- Doc sync: `src/hir/mod.rs` module docs (no de-elaboration section);
  `.ai/compiler.md` pipeline description — E's docs pass finalizes.
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Delete or keep the `deelaborate` machinery behind `#[cfg(test)]` for test
  ergonomics?** Recommend DELETE — tests asserting over HIR directly is the
  point; a test-only backward path is how backward paths return.

## Corrections

<Filled in during execution.>

## Summary

The last backward edge dies, and with it the `parse↔name` load-bearing seam.
Risk is walk breadth, not design — the port recipe has landed four times. The
review's dual-resolve/diagnostic-streaming observations are recorded as
separate work, not braided in.
