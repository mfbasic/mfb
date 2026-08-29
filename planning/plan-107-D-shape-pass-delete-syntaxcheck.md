# plan-107-D: complete hir::shape for the erased-evidence residue; DELETE src/syntaxcheck

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-107-C and plan-107-E (every pure (V) rule lives in
ir::verify; the shape pass exists with its typing seam and the
named-argument cluster; only the (S) residue, the split rules, the (I)
cleanups and the shell remain in syntaxcheck).

Move the erased-evidence residue — the rules whose constructs total lowering
destroys (per A's verdicts, plan-107-A §2) — into the `ir::shape` pass E
created, land the split (V/S) rules with both halves at once, relocate the
(I) items to their real homes, then **delete `src/syntaxcheck/` entirely**
and retire the dual-checker machinery: `RELOCATED_TO_IR_VERIFY`,
`syntaxcheck::report`'s skip logic, and the two-stream concatenation. The end
state is the one `rules/mod.rs` names: **`ir::verify` is the single semantic
checker**, plus one explicitly-scoped pre-lowering shape pass whose every rule
carries a one-line justification of what lowering erases.

See plan-107-A for shared prerequisites, gate policy, and the census.

References:

- `src/cli/build/mod.rs:373-414` — the split comment this letter deletes
  (replaced by the shape-pass rationale).
- `src/rules/mod.rs:17-22` — the merge contract; post-D the render is
  shape-stream + verify-stream (two streams still, but one is small and
  named for what it is).
- `planning/Compiler Pipeline.md:47-48` — Rec #3's `debug_assert!`-guarded
  dual-list hazard: dies with the list.
- `src/audit/mod.rs:121` — the audit path runs `syntaxcheck::check_project`;
  switches with the build path.

## Prerequisites

See plan-107-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-107-C complete | 13 NATIVE rules verify-only; C's boxes ticked | NOT MET until C lands |
| plan-107-E complete | `ir::shape` wired with the typing seam; named-argument cluster + builtin-call family landed | NOT MET until E lands |

## 1. Goal

- `ir::shape` implements exactly the (S)-verdict rules and the (S) halves of
  the split rules, each with a doc comment naming the erased evidence
  ("`ExitTarget::Func` lowers to nothing — this fact does not exist in IR").
  Collected diagnostics, merged before verify's stream (preserving each
  rule's current stream position — they render from the first stream today,
  so set AND order stay stable for these).
- **`rg -n 'syntaxcheck' src/` → 0** (directory deleted; call sites gone;
  `check_project_collect`, `export_in_executable_diagnostics`, and every
  module under `src/syntaxcheck/` removed — `export_in_executable` relocated
  beside the shape pass per its (I) verdict; `PACKAGE_INVALID`'s metadata
  validation relocated to the package decode boundary).
- `RELOCATED_TO_IR_VERIFY` and the skip logic **deleted** (with no second
  emitter, the sole-rejecter list is meaningless — and Rec #3's silent-release
  hazard is gone structurally, which beats hardening it). verify's
  `collect_source_diagnostics` emits every rule on the source path.
- Both seams (build + audit) run: `ir::shape` on the concrete HIR +
  `ir::verify` on the IR. Nothing else checks anything.

### Non-goals (explicit constraints)

- Per plan-107-A: codegen byte-identical; diagnostic sets unchanged; order
  changes only where a rule's stream membership changed (re-pinned in the
  commit that moves it) — the pure-(S) moves keep first-stream position and
  are order-neutral; the split rules' (V) halves move stream and re-pin.
- The dual-run topology (a pre-lowering pass + verify both running to
  completion) is KEPT — it is now the honest minimum, not debt.
- Resolver/monomorph short-circuit behavior unchanged (separate work).

## 2. Current State (entering D)

Post-C/E, `src/syntaxcheck/` contains: the (S) rules below, the walk driving
them, its inference (`inference.rs`, needed only until the last typed (S)
rule moves), package-metadata readers, and dead weight from the departed
rules. Its size at D's kickoff is the first measurement.

### The residue (from plan-107-A §2)

| Row | Code | Half moving to shape | Typing? | Fixtures |
|---|---|---|---|---|
| 3 | EXIT_FUNC_FORBIDDEN | whole | no | 1 |
| 4 | EXIT_SUB_IN_FUNC | whole | no | 1 |
| 6 | MONEY_INEXACT_FLOAT_LITERAL (Warn) | whole | yes (Money operand) | 2 |
| 23 | TESTING_EXPECT_ARITY | whole | no | 0 → `mfb test` fixture |
| 24 | TESTING_EXPECT_CODE_TYPE | whole | yes | 0 → fixture |
| 25 | TESTING_EXPECT_INCOMPARABLE | whole | yes (`=` acceptance) | 0 → fixture |
| 26 | TESTING_EXPECT_NOT_PRINTABLE | whole | yes | 0 → fixture |
| 27 | TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE | whole | no (canonical callee) | 0 → fixture |
| 28 | TESTING_EXPECT_TYPE_MISMATCH | whole | yes | 0 → fixture |
| 33 | TYPE_INLINE_TRAP_FALLS_THROUGH | whole | no (flow) | 1 |
| 38 | TYPE_RECOVER_OUTSIDE_INLINE_TRAP | whole | no | 1 |
| 41 | TYPE_SUB_CANNOT_RETURN_VALUE | whole | no | 0 → fixture (or moot if unparseable) |
| 22 | SUB_RETURN_FORBIDDEN | bare-`RETURN` form (verify keeps the valued form) | no | 3 |
| 31 | TYPE_DUPLICATE_FIELD | constructor named-arg form (verify keeps WITH) | no | 0 → fixture (both forms) |
| 39 | TYPE_RECOVER_TYPE_MISMATCH | the two arity forms (verify keeps mismatch) | yes (success type is Nothing?) | 1 |
| 46 | UNREACHABLE_AFTER_EXIT | after EXIT SUB / EXIT FUNC (verify keeps loop + PROGRAM forms) | no | 3 |
| 49 | TYPE_READ_ONLY_RECORD_CONSTRUCTOR | the `Error`/`ErrorLoc` constructor form (lowering synthesizes `Constructor{Error}` itself; A Corrections C-split-49); verify keeps the compiler-owned form and gains the `AttributedString` form | no | 4 |
| 5 | EXPORT_IN_EXECUTABLE | (I) the build-boundary fn moves beside the pass, unchanged | no | 1 |
| 20 | PACKAGE_INVALID | (I) metadata validation → decode boundary (`cli/build/packages.rs` / `manifest::package`) | — | 0 (unit tests) |
| 1, 2, 34 | AUGMENTATION_FAILED, CARGO_MANIFEST_DIR, TYPE_INLINE_TRAP_ON_INLINED_BUILTIN | (I) test-only strings; die with syntaxcheck's tests | — | 0 |

### Measured populations

| What | Count | Command |
|---|---|---|
| (S) rules to move | 12 whole + 4 halves + 2 relocations | table above |
| syntaxcheck residual size at kickoff | measure | `find src/syntaxcheck -name '*.rs' \| xargs wc -l` |
| syntaxcheck call sites to remove | measure at kickoff | `rg -n 'syntaxcheck' src/ --type rust -l` (48 files mention it 2026-08-29, mostly comments) |
| RELOCATED list + skip-logic sites | list + `syntaxcheck::report` asserts + `verify::collect_source_diagnostics` filter + `rules/mod.rs` references + `verify/tests.rs:7361` | `rg -n 'RELOCATED_TO_IR_VERIFY' src/` |

### Verified properties

- **The (S) rules' evidence exists in HIR** — HIR mirrors the AST 1:1
  (named `HirCallArg::Named`, `ExitTarget` flavors, `HirExpression::Trapped`,
  `HirStatement::Recover`, lambdas, `Number(text)` with suffix). VERIFIED by
  A (`src/hir/mod.rs:174-400`).
- **Pre-monomorph shapes**: none of the residue needs the generic HIR —
  every rule reads a construct that monomorph preserves (EXIT/RETURN/RECOVER
  statements, literal spellings, assertion calls, SUB return annotations).
  VERIFIED by A's table: no row's evidence is an overload's pre-mangling
  parameter names (the named-argument rules, E, resolve against the
  post-monomorph callee whose params the mangled signature still carries).
  `EXPORT_IN_EXECUTABLE` reads the original AST today and keeps doing so.

## 3. Design Overview

Port the (S) rules into E's `ir::shape` (the recipe from `6db8e040b`'s
ports: mirrored variants, structural type reads; typed rules through the
seam), one commit per rule; land the split rules both-halves-at-once; move
the (I) items; then delete: `src/syntaxcheck/` wholesale, its call sites,
the list, the skip logic, and the split comment — replaced by the shape-pass
rationale. Then the docs pass.

**Risk concentration:** deletion fallout — helpers syntaxcheck still exports
that something else quietly uses (`syntaxcheck::testutil`, the `Type` alias
re-exports, `is_builtin_nominal`…). Mitigation: `rg -n 'syntaxcheck' src/` →
0 is the acceptance, and the compiler enforces it; anything that breaks is a
dependency to relocate explicitly, not silently.

### Rejected alternatives

- **Keep a gutted syntaxcheck as the shape pass's home.** Rejected: the name
  is the misdirection ("syntaxcheck" doing semantics confused this codebase's
  own review); a fresh, small, correctly-named module with per-rule
  justifications is the maintainable end state.
- **Fold the shape rules into `elaborate`.** Rejected (recorded in A):
  validation coupled into construction makes both harder to reason about.

## Compatibility / Format Impact

None to codegen/wire. Diagnostic sets unchanged; order stable for pure-(S)
moves; split rules re-pin (listed).

## Phases

### Phase 1 — the (S) ports + split rules

- [ ] Fixtures first for the zero-fixture rows (23–28 as `mfb test`
      fixtures; 31 both forms; 41).
- [ ] Pure-(S) rules, one commit each (rows 3, 4, 6, 23–28, 33, 38, 41), each
      with its erased-evidence doc line; corpus SAME (order-neutral) per
      commit.
- [ ] Split rules, one commit each (rows 22, 31, 39, 46): shape half + verify
      half confirmed/ported + list entry + syntaxcheck deletion; corpus
      set-equal, reorders listed.
- [ ] (I) relocations: `export_in_executable_diagnostics` moved beside the
      pass; `PACKAGE_INVALID` metadata validation moved to the decode boundary
      with its unit tests (prove which sites the resolver already shadows —
      moot with evidence for those).
- [ ] Tests: corpus set-equal per commit; full suite.

Acceptance: every (S) rule fires from `ir::shape`; syntaxcheck's copies
deleted; corpus set-equal (order-identical for pure-(S) moves).
Commit: — (per rule)

### Phase 2 — delete src/syntaxcheck + the split machinery

- [ ] Delete `src/syntaxcheck/` and all call sites; relocate any straggler
      helper some other module used (explicitly, with a note here).
- [ ] Delete `RELOCATED_TO_IR_VERIFY`, the `syntaxcheck::report` skip logic,
      the `collect_source_diagnostics` filter, the `rules/mod.rs` split
      references and the parity test's list dependency; replace
      `build/mod.rs:373`'s split comment with the shape-pass rationale.
- [ ] Tests: full suite; the whole diagnostic corpus; `artifact-gate all`.

Acceptance: `rg -n 'syntaxcheck\|RELOCATED_TO_IR_VERIFY' src/` → 0; corpus
set-equal; gate byte-identical; full suite green.
Commit: —

### Phase 3 — docs pass + closing census

- [ ] `.ai/compiler.md` (checking topology: shape + verify),
      `.ai/testing-gates.md` (check-pass topology + the harness),
      `.ai/resources-packages.md` (LINK validation home), spec
      `02_frontend.md` — the review's "syntaxcheck" descriptions replaced;
      `AGENTS.md`/memory index references checked.
- [ ] Closing census recorded here: rule count in `ir::shape` (each with its
      justification line), rule count in verify, zero elsewhere; the
      plan-20-Z era formally closed.

Acceptance: docs updated; census recorded; full suite; gate; test-accept; fmt
both crates.
Commit: —

## Validation Plan

- Tests: per-rule corpus runs; full suite; package-path tests from B/C/E stay
  green.
- Coverage check: A's fixture-per-rule requirement carried through the (S)
  set.
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: Phase 3.
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Generic vs concrete HIR for the shape pass** — concrete for every rule
  (A's evidence, §2 Verified properties); `EXPORT_IN_EXECUTABLE` keeps its
  original-AST read.

## Corrections

- **2026-08-29 (from A's audit).** The shape pass is created in E (with its
  typing seam and the named-argument cluster), not here; D completes it. The
  residue is 12 whole rules + 4 split halves, seven of which need expression
  typing (A Corrections C-shape-typing). `PACKAGE_INVALID` and
  `EXPORT_IN_EXECUTABLE` are (I) relocations, not shape rules.

## Summary

The deletion letter: a 14k-line misnamed subsystem is replaced by one small,
honestly-named shape pass whose every rule justifies its own existence, and
`ir::verify` becomes the single semantic checker — closing the relocation
trajectory this codebase started in plan-20 and the review's Rec #3 hazard
with it.
