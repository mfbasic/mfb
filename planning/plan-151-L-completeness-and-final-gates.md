# plan-151-L: Whole-registry completeness and final gates

Last updated: 2026-09-23
Effort: medium (1h–2h) of review, plus final suite wall time; estimate.
Depends on: plan-151-K; all earlier letters must be complete.

Close the migration: every public builtin overload has reviewed cost metadata,
every function page renders it, and future omissions fail the registry tests.
The authoritative design is [plan-151-A](plan-151-A-cost-schema-and-rendering.md).
This is the final letter; the feature is not complete at an earlier boundary.

## Prerequisites

Recheck A's prerequisites and every preceding live or archived letter's task boxes,
Commit lines and acceptance evidence. Command:
`rg -n '^- \[ \]|^- \[~\]|^Commit:' planning/plan-151-[A-K]-*.md planning/completed/plan-151-[A-K]-*.md`
using only the paths that exist (`rg --files planning` discovers them). No unchecked
or partial task and no empty Commit line may remain. At authoring: NOT MET; A–K
are not implemented. Re-run instead of acting on this snapshot.

## 1. Goal / non-goals

A fresh registry walk proves complete public-overload and public-function-page
coverage, including general/testing and helper-generated overloads. Metadata
remains documentation-only. No benchmark redesign, codegen improvement, user-code
cost inference, native timing certification or `.mfp` change.

## 2. Current state and population

A's source census is the scheduling evidence; expanded final counts are deliberately
not guessed. `cost_census` added by A measures the live registry at this stage.
Read `src/cli/man.rs:render_package_all_markdown` and `render_all_markdown` when
checking coverage: top-level --all is not the whole function-page denominator.
`tests/gate/golden.rs:artifact_gate_all` already drives the full artifact gate under
cargo test; do not run another full artifact gate afterward.

## 3. Design

Delete A's completed-package allow-list. Keep `Implementation.cost: Option<Cost>`
with its FINAL meaning: public functions must be Some for every overload;
internal-only functions may be None. This is not a default or unknown public cost.
Add an unconditional test `every_public_overload_has_cost` and a real registry
renderer test `every_public_function_renders_cost`. Both assert a nonempty scope,
walk all packages, and name missing function/signature/overload rows in failures.

No public Cost section may contain placeholder text or unexplained provider
classification. ProviderDependent/NoFiniteBound require a reviewed reason and
source evidence; they are not missing data. Review error-path bounds and callback
call sites, not merely successful examples. Tests enforce structure, evidence
navigation and presence; the ledger remains the human evidence for mathematical
correctness. Do not advertise the tests as automatic complexity verification.

## Compatibility / Format Impact

Man text is expected to change. Generated programs, diagnostics, wire formats,
registry selection and declared errors are expected to remain unchanged. Unexpected
artifact diffs require inspection of one fixture to localize the cause. Never
rebaseline a whole golden file to make this documentation change pass.

## Phases

### Phase L1 — Remove the migration escape and reconcile all rows

- [ ] Remove temporary coverage allow-list; add unconditional public overload/page
      coverage tests and preserve internal-only exclusion.
- [ ] Run the final all-package `cost_census`; reconcile every final overload/case
      against `planning/plan-151-cost-review.md`. Any package/function added since
      its letter was reviewed belongs to that letter's scope and must be reviewed
      before this gate can pass. Update measured counts and corrections.
- [ ] Review `src/codegen/registry/cost.rs` validation: missing public cost, empty
      cases, bad wait/callback parameter roots, duplicate variables, empty conditional
      assumptions, malformed/missing evidence and all new schema variants have
      negative tests. Keep the expected/amortized versus worst-case distinction.
- [ ] Confirm none of the fields feeds resolution, error classification, codegen,
      optimizer decisions or serialization (`rg -n '\bcost\b|CostCase|WorkBound'
      src` and read each non-documentation consumer). Add a source review record;
      grep alone is not proof of noninterference.

Acceptance: whole-registry public coverage is complete and dynamically enforced.
Check: `MFB_COST_REQUIRE_COMPLETE=1 cargo test --bin mfb cost_census -- --nocapture`
→ zero missing public overloads, nonzero totals, exit 0 (estimate 5 min).
Commit: —

### Phase L2 — Reader-facing review and documentation consistency

- [ ] Render the complete public registry via the renderer test (including
      general/testing); ensure every page has exactly one Cost section and overload
      numbering refers to existing signatures. Equal public contents group despite
      differing evidence; unequal contents do not group.
- [ ] Render actual CLI pages for collections append/get/any, thread send, os sleep,
      http route/handleRequest, general toString, testing expectEqual and tooling cost.
      Check legibility at the normal terminal width and narrow output using the
      renderer's existing width facilities in tests. No raw paths or internal symbols.
- [ ] Update `src/docs/spec/tooling/10_builtin-costs.md` to final coverage policy,
      removing the temporary migration description from normative behavior.
      Check `tooling/spec.md`, `07_cli-reference.md`, `.ai/man-content.md` and the
      man tooling overview/guide agree. Do not copy per-function cost tables into spec.
- [ ] Review the cost ledger's source evidence and benchmark-gap notes. Every row
      must be resolved. A missing benchmark is acceptable with source proof;
      unresolved complexity is not a completed row.

Acceptance: the public output is complete, correctly scoped and does not claim a
hard deadline from computational work or timeout metadata.
Check: `cargo test --bin mfb cost_` → all final structure and rendering tests pass
(estimate 5–10 min).
Commit: —

## Validation Plan

Run on the local macOS development host, sequentially. Durations below are planning
estimates, not measured timings. Do not run Cargo while a harness uses its binary.
No remote execution is needed for this documentation/registry feature. If review
reveals a needed runtime change, it is separate bug work with the applicable gates,
not a reason to silently expand the cost implementation.

- `cargo build --bin mfb --tests` → successful current compiler/test compilation
  (estimate 10 min).
- `target/debug/mfb man collections append` → Cost section with reviewed cases
  and qualified growth statements (estimate <1 min; actual CLI runtime proof).
- `target/debug/mfb man tooling cost` → readable cost-model guide (estimate <1 min).
- `cargo test --bin mfb spec` → embedded spec checks pass (estimate 5 min).
- `scripts/spec-census.sh --citations tooling` → no unresolved citations in the
  owning tooling topics (estimate 2 min).
- `scripts/spec-census.sh --links tooling` → no unresolved man/spec links in the
  owning tooling topics (estimate 2 min).
- `MFB=./target/debug/mfb scripts/man-census.sh --memory-scope` → inspect complete
  report, no new unclassified hits from Cost sections/guide (estimate 3 min).
  Existing findings must be distinguished by a recorded baseline rather than
  silently changing the scanner or granting a new vocabulary exemption.
- `cargo test` → full suite passes ONCE, including `tests/gate/golden.rs`
  (estimate 1–3 hours; a smaller scoped run cannot provide the repository's required
  final integration/regression coverage across the broadly edited registry).
- `scripts/test-accept.sh target/debug/mfb target/accept-actual` → acceptance passes
  ONCE, after Cargo finishes (estimate 30–60 min; catches diagnostic/fixture regressions
  not covered by a renderer test or the artifact gate). Check harness locks first;
  exit 98 is a refusal, never a pass or regression verdict.

If a golden/test appears wrong, apply AGENTS.md's historical intent, protected
behavior, dependent-contract and independent-proof requirements before changing it.
The full suite precedes any golden rebaseline. This feature expects none.
Archive completed letters under `planning/completed/` rather than deleting them;
archive the resolved cost-review ledger with the final letter, preserving links.

## Open Decisions

None.

## Corrections

None at authoring. Record final denominator changes and any corrected cost claims.

## Summary

Completeness means every public overload is reviewed, every corresponding page
renders its cost and future omissions fail. Passing compilation or a few sample
pages alone does not establish completion.
