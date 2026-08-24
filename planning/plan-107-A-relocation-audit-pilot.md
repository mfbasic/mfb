# plan-107-A: Single-checker relocation — evidence audit + pilot rules

Last updated: 2026-08-24
Overall Effort: huge (>3d) — the whole plan-107 feature
Effort: large (3h–1d)
Depends on: nothing within 107 (plan-106 complete is the gate)

Finish the codebase's own declared end state — `rules/mod.rs`: "the eventual
single-checker (`ir::verify` traversal) end state" — by relocating **every
remaining semantic rule** out of `src/syntaxcheck/` into `ir::verify`, moving
the small set whose evidence lowering erases into a minimal pre-lowering shape
pass, and then **deleting `src/syntaxcheck/` entirely** along with the
dual-checker split machinery (`RELOCATED_TO_IR_VERIFY`, the skip logic, the
two-stream merge).

This also closes a real security gap class: today a rule still implemented
only in syntaxcheck does **not** guard decoded `.mfp` packages (verify is the
sole checker on that path — `verify/mod.rs` module docs, review finding
PKG-02). Every rule relocated in this plan starts guarding packages too.

This sub-plan is the **lead document for plan-107**. Roadmap (letter order =
implementation order):

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | Per-rule evidence audit of all 46 remaining codes; the relocation recipe priced by 3 pilot rules landed end-to-end | large |
| **B** | The general semantic cluster relocated | large |
| **C** | The `NATIVE_*`/LINK cluster + TESTING-assert family (per A's verdicts) | large |
| **D** | Erased-evidence residue → `hir::shape`; **DELETE `src/syntaxcheck/`**; retire the split machinery; single-stream rendering | large |

References:

- `src/ir/verify/mod.rs:71` — `RELOCATED_TO_IR_VERIFY` (74 entries:
  `awk '/RELOCATED_TO_IR_VERIFY/,/^\];/' … | grep -cE '"'` → 74) and the
  sole-rejecter mechanism (`syntaxcheck::report` skips listed codes) — the
  proven incremental relocation machinery this plan drives to completion.
- `src/cli/build/mod.rs:359-370` — the split rationale comment (names the
  erased-construct classes: named arguments, EXIT flavors, inline-trap
  boundaries).
- `src/rules/mod.rs:17-22` — the merge-order contract and the declared
  single-checker end state.
- `planning/Compiler Pipeline.md:47-48` — the review findings this plan
  discharges (dual-list `debug_assert!` hazard dies with the list itself).
- The plan-20-E..I history (`planning/completed/`) — the per-rule
  reproduction discipline this plan continues.

## Prerequisites

Shared by every plan-107 letter; stated once here.

| Must be true | Command | Status |
|---|---|---|
| plan-106 complete | plan-106-A..E archived; `rg -n 'deelaborate' src/` → 0; `rg -n 'enum Type' src/syntaxcheck/` → 0 | NOT MET — plan-106 not started |
| Feature worktree + fresh baselines | as plan-104-A §Prerequisites (gate + suite + bench) | NOT MET — run first |

plan-106 is a hard gate, not a preference: post-106 the rules are
`ParameterType`-typed on HIR, so each relocation is a **transcription** into
verify's (also typed, post-106-B) environment instead of a string→typed
rewrite; and the erased-evidence residue lands in a shape pass that reads HIR,
which only exists as the checker's input after 106-D.

## 1. Goal

- Every one of the 46 not-yet-relocated codes
  (`comm -23` of syntaxcheck's emitted codes vs the relocated list → 46 at
  plan-writing; the exact list is recorded in §2) is classified with
  **evidence**, and 3 pilot relocations land end-to-end proving the recipe and
  its per-rule cost.
- The **diagnostic gate policy** for the whole plan is established and
  encoded in a harness (see §3) — because this plan is NOT diagnostics-order
  neutral and pretending otherwise would corrupt every later letter.

### Non-goals (explicit constraints)

- No change to compiled output for accepted programs — `artifact-gate all`
  byte-identical throughout every letter (codegen is untouched by rule
  relocation).
- No rule is weakened, merged, or reworded: same code, same message text, same
  file:line, per fixture — only the **stream a rule renders in** (hence
  relative order in multi-error fixtures) changes, and only when that rule
  relocates.
- Resolver/monomorph diagnostics (print-and-short-circuit) are NOT restructured
  here (the review's Rec #5 stays separate work).

## 2. Current State

The dual-checker split runs both passes to completion and concatenates
streams (`build/mod.rs:359`, `rules/mod.rs:17`). 74 rules are verify's;
46 codes still render from syntaxcheck. The remaining set, measured
(`grep -rhoE '"[A-Z][A-Z_]+_[A-Z_]+"' src/syntaxcheck/ | sort -u` minus the
relocated list):

```
AUGMENTATION_FAILED CARGO_MANIFEST_DIR EXIT_FUNC_FORBIDDEN EXIT_SUB_IN_FUNC
EXPORT_IN_EXECUTABLE MONEY_INEXACT_FLOAT_LITERAL NATIVE_ABI_NO_RESULT
NATIVE_ABI_RESULT_MARKER NATIVE_ABI_UNBOUND_PARAM NATIVE_ABI_UNBOUND_SLOT
NATIVE_ABI_UNKNOWN_CTYPE NATIVE_BIND_IN_INVALID NATIVE_CONST_OUT
NATIVE_CONST_UNKNOWN_SLOT NATIVE_CPTR_ESCAPE NATIVE_CSTRUCT_ESCAPE
NATIVE_CSTRUCT_INVALID NATIVE_FREE_INVALID NATIVE_STRUCT_FIELD_MISMATCH
PACKAGE_INVALID RESOURCE_SHADOWS_BUILTIN SUB_RETURN_FORBIDDEN
TESTING_EXPECT_ARITY TESTING_EXPECT_CODE_TYPE TESTING_EXPECT_INCOMPARABLE
TESTING_EXPECT_NOT_PRINTABLE TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE
TESTING_EXPECT_TYPE_MISMATCH TYPE_COLLECTION_OWNERSHIP_VIOLATION
TYPE_DUPLICATE_ARGUMENT_NAME TYPE_DUPLICATE_FIELD TYPE_INLINE_TRAP_DEAD_HANDLER
TYPE_INLINE_TRAP_FALLS_THROUGH TYPE_INLINE_TRAP_ON_INLINED_BUILTIN
TYPE_INLINE_TRAP_REQUIRES_FALLIBLE TYPE_ISOLATED_NOT_VISIBLE
TYPE_LAMBDA_CAPTURE_UNSUPPORTED TYPE_RECOVER_OUTSIDE_INLINE_TRAP
TYPE_RECOVER_TYPE_MISMATCH TYPE_RESULT_NOT_USER_VISIBLE
TYPE_SUB_CANNOT_RETURN_VALUE TYPE_THREAD_NOT_SENDABLE TYPE_TRAP_FALLTHROUGH
TYPE_UNKNOWN_ARGUMENT_NAME TYPE_UNKNOWN_VALUE UNREACHABLE_AFTER_EXIT
```

Expected classification (VERIFY per rule in Phase 1 — this table is the
hypothesis, not the audit):
- **(V) relocatable to `ir::verify`** — semantic rules whose facts survive
  lowering (e.g. `TYPE_UNKNOWN_VALUE`, `TYPE_THREAD_NOT_SENDABLE`,
  `TYPE_RECOVER_TYPE_MISMATCH`, `RESOURCE_SHADOWS_BUILTIN`,
  `TYPE_COLLECTION_OWNERSHIP_VIOLATION`, the `NATIVE_*` family — LINK data
  rides to IR verbatim and `verify/link.rs` already checks siblings).
- **(S) shape residue** — evidence erased by lowering (named-argument rules:
  lowering normalizes to positional; `EXIT_*`/`SUB_RETURN` flavors:
  normalized; the `TYPE_INLINE_TRAP_*`/`RECOVER`/`TRAP_FALLTHROUGH` family:
  restructured; `TESTING_EXPECT_*`: the `expect()` calls are desugared during
  lowering; `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`: lambdas become closures;
  `MONEY_INEXACT_FLOAT_LITERAL`: literal spelling; `UNREACHABLE_AFTER_EXIT`:
  source shape).
- **(I) infra/false-positive** — `CARGO_MANIFEST_DIR` (an env var caught by
  the grep), `AUGMENTATION_FAILED`/`PACKAGE_INVALID`/`EXPORT_IN_EXECUTABLE`
  (build-boundary codes — classify where they actually render from).

### Verified properties

- **The relocation mechanism works and is golden-guarded** — 74 rules landed
  through it (the list + skip logic + per-rule fixture verification;
  plan-20-E..I history). VERIFIED (landed history).
- **Order changes are confined and mechanical**: moving a rule changes only
  which stream it renders in; a fixture tripping a single rule sees NO golden
  change; multi-error fixtures see a deterministic reorder. UNVERIFIED as a
  blanket claim — Phase 1's pilot measures how many fixtures per rule actually
  reorder (record the count).
- **Verify's typed inference (post-106-B) covers the moved rules' needs** —
  UNVERIFIED; the audit records, per (V) rule, which typing facts it needs and
  whether verify's `infer_type` already derives them (gaps become explicit
  port tasks in B/C, not surprises).

## 3. Design Overview

**The gate policy (applies to every letter — this is the plan's spine):**
- Accepted programs: `artifact-gate all` byte-identical, always.
- Rejected programs: per-fixture **diagnostic-set equality** — same
  (code, file, line, message) multiset before and after each relocation;
  ORDER may change only on fixtures that trip the relocated rule, and each
  affected golden is regenerated **deliberately and listed in the commit**.
  A harness script (`scripts/diag-set-diff.sh`, built in Phase 1) diffs the
  set-normalized diagnostics of every `*-invalid` fixture pre/post so a
  wording or line drift can never hide inside an expected reorder.
- Package path: for each (V) rule with a reachable package-path shape, a
  crafted-`.mfp` or package fixture proving the rule now fires there (the
  security payoff made testable).

**Relocation recipe per rule** (the pilots prove it): implement in the right
`verify/` module reading typed IR (+ port any missing inference fact) → add
the code to `RELOCATED_TO_IR_VERIFY` (syntaxcheck's copy goes silent) → run
the corpus with the harness → regenerate the listed goldens → delete the
syntaxcheck implementation → corpus again (proves the list entry, not luck,
did the silencing).

**Risk concentration:** wording/line fidelity — verify derives locations from
IR spans (plan-20-A infrastructure), and any divergence is a golden diff the
harness pins to the exact fixture.

### Rejected alternatives

- **Wholesale move (all 46 in one diff).** Rejected: un-reviewable golden
  churn; a single wording slip hides in thousands of reordered lines. The
  74-rule history was incremental for this reason.
- **Annotate IR to carry erased evidence so (S) rules relocate too.**
  Rejected: fattens the IR and the `.mfp` wire surface for validation-only
  data; a 200-line HIR shape pass is strictly cheaper (D).

## Compatibility / Format Impact

None to codegen or wire formats. Diagnostic ORDER on multi-error fixtures
changes deliberately per relocation (goldens re-pinned); set never changes.

## Phases

### Phase 1 — the audit + the harness

- [ ] Build `scripts/diag-set-diff.sh` (set-normalized per-fixture diagnostic
      compare over the `*-invalid` corpus); prove it flags a planted wording
      change and passes a planted reorder.
- [ ] Audit all 46 codes: for each, read the syntaxcheck implementation and
      the lowering path of its construct; record verdict (V)/(S)/(I) with the
      evidence (what survives into IR), the inference facts needed, and the
      count of corpus fixtures that trip it. Replace §2's hypothesis table
      with the verdicts. This table SETS letters B/C/D's scopes — re-scope
      them in place if the split shifts.

Acceptance: harness proven; the verdict table complete with no hypothesis
rows.
Commit: —

### Phase 2 — three pilot relocations

- [ ] Relocate 3 (V)-verdict rules of different shapes (one expression-level,
      one decl-level, one needing an inference-fact port) end-to-end via the
      recipe; record per-rule cost and reordered-fixture counts.
- [ ] Tests: full corpus via the harness; package-path fixture for at least
      one pilot.

Acceptance: 3 rules live in verify, syntaxcheck impls deleted, corpus
set-equal, goldens re-pinned and listed, `artifact-gate all` byte-identical.
Commit: —

## Validation Plan

- Tests: the harness across the full `*-invalid` corpus; pilots' package-path
  fixtures; full suite.
- Coverage check: the audit records fixture counts per rule — a rule with ZERO
  corpus fixtures gets one written BEFORE its relocation (no unguarded moves).
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: none yet (D owns it).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Where the (S) shape pass lives** (D's letter): `src/hir/shape.rs` run
  right after `elaborate`, collected into the same diagnostic merge.
  Recommended; the alternative (checks inside `elaborate` itself) couples
  construction with validation.

## Corrections

<Filled in during execution.>

## Summary

The audit is the plan: 46 verdicts with evidence, a set-equality harness that
makes reorder churn safe to review, and three priced pilots. B/C are then
production-line work, and D deletes a 14k-line subsystem plus the
`debug_assert!`-guarded split the review flagged.
