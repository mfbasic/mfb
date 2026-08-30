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
| 14 | NATIVE_CONST_UNKNOWN_SLOT | the "not a constant the compiler can fold" form — the pin's expression is folded away by lowering (C Corrections); verify keeps the unknown-slot form | no | 1 (`native-const-unfoldable-invalid`) |
| 18 | NATIVE_FREE_INVALID | the deallocator-signature sub-condition of the "malformed FREE" form — `IrFree` carries slot + symbol only (C Corrections); verify keeps the `AS RES` producer form and the empty-symbol form | no | 2 |
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

- [x] Fixtures first for the zero-fixture rows (23–28 as `mfb test`
      fixtures; 31 both forms; 41).
      (23–28: `tests/syntax/testing/testing-assert-invalid` already carries
      every TESTING_EXPECT_* rule but NOT_PRINTABLE in its
      `golden/testing_assert.testrun` (`mfb test` proof; `diag-set-diff.sh`
      now reads `.testrun` goldens, C-harness-test-verb) — the NOT_PRINTABLE
      case (`expectEqual(m, m)` on a Map) added and its `.testrun`/`.ast`
      goldens regenerated with the pre-port binary; 41: moot, see C-41-moot;
      31: `tests/syntax/types/types-duplicate-field-invalid` carries both
      forms (constructor + WITH), golden generated with the pre-port binary.)
- [x] (added) Fixture for row 18's deallocator-signature form — the two
      NATIVE_FREE_INVALID fixtures the residue counted are the parser's
      (`bug90_free_missing_symbol`) and verify's `AS RES` form; the existing
      `native-free-invalid` fixture is a PARSE error (its `RETURN` sits inside
      the FREE block) and never reached the checker. Added
      `tests/syntax/native/native-free-deallocator-invalid` (`ABI (ptr CInt32)
      AS CVoid`), golden from the pre-port binary.
- [~] Pure-(S) rules, one commit each (rows 3, 4, 6, 23–28, 33, 38, 41), each
      with its erased-evidence doc line; corpus SAME (order-neutral) per
      commit.
      (Rows 3, 4, 33, 38 landed — in ONE commit with rows 22, 39, 46 and E's
      C-override-typing fixup, not one each: see C-one-commit. Row 41 moot
      (C-41-moot). Rows 23–28 (the six TESTING_EXPECT_* rules) landed as one
      commit: `Walker::check_expect_call` is the checker's transcription over
      the HIR call (`expand_expect` rewrites the assertion into comparisons +
      FAIL / a trap guard before the IR exists), with the `=` acceptance rule
      (`is_comparable`, which needed `TypeShape.fields`/`is_enum` and the
      checker's resource registry — builtin + native `LINK` + imported
      `RESOURCE_TABLE` names, now a `collect_diagnostics` input) and
      `is_printable` ported; corpus 522 same / 0 reordered / 0 set-diff.
      Row 6 (MONEY_INEXACT_FLOAT_LITERAL, Warn) landed: the literal's
      SPELLING is the evidence (`1.08` vs `1.08f` lower to the same Float
      const); corpus 522 same. All pure-(S) rows done.)
- [x] Split rules, one commit each (rows 22, 31, 39, 46): shape half + verify
      half confirmed/ported + list entry + syntaxcheck deletion; corpus
      set-equal, reorders listed.
      (22, 39, 46 landed with the control-flow group (C-one-commit); the
      verify halves were NOT all sound as found — C-handler-truncation (46)
      and C-recover-literal (39) record the two blind spots and their fixes.
      Corpus after the group: 517 same, 4 reordered (`control-flow/
      continue-loop-invalid`, `control-flow/exit-loop-invalid`,
      `functions/sub-value-less-invalid`, `trap/control-flow-inline-trap-
      invalid` — the fixtures carrying the relocated rules; all four goldens
      regenerated as pure line moves), 0 set-diff. Row 31 landed: the
      constructor form is shape's (lowering reorders named arguments into
      field order — the last spelling wins — so the repetition is gone),
      gated on a declared visible record as the checker's
      `check_constructor_arguments` was; verify's WITH form now emits on the
      source path. Corpus: 522 same, 1 reordered (the new fixture: shape's
      constructor form, then verify's Bind-order ARITY + WITH; same three
      records), 0 set-diff.)
- [x] (I) relocations: `export_in_executable_diagnostics` moved beside the
      pass; `PACKAGE_INVALID` metadata validation moved to the decode boundary
      with its unit tests (prove which sites the resolver already shadows —
      moot with evidence for those).
      (Rows 14 + 18 (the NATIVE halves) landed first: `Walker::walk_link`
      holds the CONST "not foldable" form (`link_const_foldable`, the
      checker's `foldable`) and the FREE deallocator-signature form, skipping
      exactly the two conditions verify reports (`AS RES` producer, empty
      symbol) so nothing doubles; syntaxcheck's `check_link_block`/
      `check_link_function_in` deleted. Corpus 524 same. Row 49 landed: the
      `Error`/`ErrorLoc` form is shape's (lowering synthesizes
      `Constructor{Error}` itself), verify's `check_constructor` gained the
      `AttributedString` form (and its compiler-owned form now reaches the
      source path); syntaxcheck's three sites reduced to inference. Corpus:
      522 same, 2 reordered (`functions/func_typesystem_error_invalid`,
      `term/func_term_terminalSize_invalid` — pure moves), 0 set-diff.
      Row 5 landed: `export_in_executable_diagnostics` (+ its three tests)
      moved verbatim to `ir::shape`; the build calls it there, still over the
      original AST at the same point in the stream. Corpus 524 same.
      Row 20 landed — into the shape pass's import walk rather than the decode
      boundary (C-package-invalid-home): `Walker::check_imported_packages` +
      `validate_package_type` carry the checker's six PACKAGE_INVALID sites
      (three unreadable-table forms; the unknown-type and non-comparable-map-
      key walks over every exported type/union and every exported function
      signature). No site was moot: the decode boundary rejects an unreadable
      CONTAINER (probe: a garbage `packages/badpkg.mfp` for a declared
      dependency → `[Tampered]` + PACKAGE_INVALID before any checker), but an
      unreadable TABLE inside a well-formed container and the semantic type
      walk had only the checker. With it syntaxcheck's `report` had no caller
      left and is deleted: the module emits NOTHING. Corpus 524 same.)
- [ ] Tests: corpus set-equal per commit; full suite.

Acceptance: every (S) rule fires from `ir::shape`; syntaxcheck's copies
deleted; corpus set-equal (order-identical for pure-(S) moves).
Commit: `f2d52f271` (control-flow group + E's seam fixup); `794eada94`
(TESTING_EXPECT_*); `d77dd17fd` (MONEY_INEXACT_FLOAT_LITERAL);
`37f5c0f27` (TYPE_DUPLICATE_FIELD); `480c8e37a` (NATIVE_* halves);
`ac9c9af95` (TYPE_READ_ONLY_RECORD_CONSTRUCTOR); `32cd106e0`
(EXPORT_IN_EXECUTABLE); PACKAGE_INVALID —

### Phase 2 — delete src/syntaxcheck + the split machinery

- [x] Delete `src/syntaxcheck/` and all call sites; relocate any straggler
      helper some other module used (explicitly, with a note here).
      (`git rm -r src/syntaxcheck` (14 files); `mod syntaxcheck` gone from
      `main.rs`; the build seam merges `ir::shape` + `ir::verify` only; the
      audit seam keeps `ir::shape::check_project` and drops the checker's
      call. Stragglers: the pipeline test oracle (`testutil::check_src` /
      `accepts` / `rejects_with`) had no consumer left outside the module
      and is deleted — `ir::shape` and `ir::verify` tests use their own
      per-pass oracles and the diagnostic corpus is the pipeline-level truth;
      the checker-only helpers surfaced as dead code and went with it:
      `codegen::resource::{ResourceRegistry, ResourceInfo, ResourceKind::
      {Imported, Native}}` (its four tests rewritten over the clean-room
      registry's descriptors and the free predicates), `builtins::
      package_constant_type_name` (the registry half stays — lowering uses
      it); the HIR-domain augmentation chain (`resolver::augment_hir_project`
      + the three `augmented_hir_project`) is `#[cfg(test)]` (its remaining
      callers are the in-process tests that monomorphize a bare project); the
      two decoded wire fields nothing reads any more
      (`BinaryReprTypeField.visibility`, `BinaryReprResourceExport.
      close_may_fail`) keep a targeted allow with the reason.)
- [x] Delete `RELOCATED_TO_IR_VERIFY`, the `syntaxcheck::report` skip logic,
      the `collect_source_diagnostics` filter, the `rules/mod.rs` split
      references and the parity test's list dependency; replace
      `build/mod.rs:373`'s split comment with the shape-pass rationale.
      (All done; the filter's removal exposed one thing it had been hiding —
      C-structural-filter. The NATIVE parity test
      (`native_rule_sets_agree_between_syntaxcheck_and_verify`) is deleted
      with its premise; `collect_source_diagnostics_filters_relocated` became
      `collect_source_diagnostics_maps_rules_to_pending`. Every remaining
      mention of the module in `src/` (161 lines of comments across 42
      files) reworded: `rg -n 'syntaxcheck|RELOCATED_TO_IR_VERIFY' src/` → 0.)
- [x] Tests: full suite; the whole diagnostic corpus; `artifact-gate all`.
      (bin unit suite `cargo test --bin mfb --no-fail-fast`: 3370 passed, the
      one failure being the spec citation drift Phase 3's docs pass repairs
      (landed in the same commit, so the tree is never red); corpus 524 same /
      0 reordered / 0 set-diff after the structural filter; `artifact-gate
      all`: 1734 goldens, 0 diffs. The full workspace suite and the
      acceptance sweep run at the letter's end — recorded under Phase 3.)

Acceptance: `rg -n 'syntaxcheck\|RELOCATED_TO_IR_VERIFY' src/` → 0; corpus
set-equal; gate byte-identical; full suite green.
Commit: — (one commit with Phase 3's docs pass: the spec's `[[src/syntaxcheck/…]]`
citations fail `spec_citations_resolve` the moment the module is gone)

### Phase 3 — docs pass + closing census

- [x] `.ai/compiler.md` (checking topology: shape + verify),
      `.ai/testing-gates.md` (check-pass topology + the harness),
      `.ai/resources-packages.md` (LINK validation home), spec
      `02_frontend.md` — the review's "syntaxcheck" descriptions replaced;
      `AGENTS.md`/memory index references checked.
      (Done, and wider than listed because `spec_citations_resolve` fails on
      any `[[src/syntaxcheck/…]]` citation: every spec/man page citing or
      describing the module was repointed at the live home — `02_frontend`
      (the shape-pass section replaces the checker's), `09_modules` (rows),
      `04_ir`, `12_monomorphization`, `22_type-inference` (recast over
      `ir::lower::expression_type` / `ir::verify::compat` / `ir::shape`),
      tooling `04_audit-format` + `07_cli-reference`, language `04_types`,
      `06_functions`, `07_subs`, `12_collections`, `13_modules-and-packages`,
      `14_memory-semantics`, `16_threads`, `17_native-libraries`, threading
      `01_source-model`, `02_isolation`, `08_queue-semantics`, stdlib
      `02_datetime`, `09_vector`, `man/link/package`; `.ai/collections.md`,
      `.ai/codegen-invariants.md`, `.ai/resources-packages.md` (the
      augmentation-chain and `ARGUMENT_CHECKED_PACKAGES` guidance),
      `.ai/compiler.md` + `.ai/testing-gates.md` gained the topology and
      `diag-set-diff.sh` paragraphs; `AGENTS.md` never named the module; the
      memory index line and four memory files updated/annotated. Verified:
      `rg -n 'syntaxcheck|RELOCATED_TO_IR_VERIFY' .ai AGENTS.md src/docs` → 0;
      `cargo test --bin mfb docs::` 26 passed.)
- [x] Closing census recorded here: rule count in `ir::shape` (each with its
      justification line), rule count in verify, zero elsewhere; the
      plan-20-Z era formally closed.
      (`/tmp/p107-census.sh` — rule names extracted from every `emit(` site:
      **`ir::shape` emits 24 rules**, 14 of them shape-only
      (EXIT_FUNC_FORBIDDEN, EXIT_SUB_IN_FUNC, MONEY_INEXACT_FLOAT_LITERAL,
      PACKAGE_INVALID, the six TESTING_EXPECT_*, TYPE_DUPLICATE_ARGUMENT_NAME,
      TYPE_INLINE_TRAP_FALLS_THROUGH, TYPE_RECOVER_OUTSIDE_INLINE_TRAP,
      TYPE_UNKNOWN_ARGUMENT_NAME) and 10 split with verify by form
      (NATIVE_CONST_UNKNOWN_SLOT, NATIVE_FREE_INVALID, SUB_RETURN_FORBIDDEN,
      TYPE_CALL_ARGUMENT_MISMATCH, TYPE_CALL_ARITY_MISMATCH,
      TYPE_DUPLICATE_FIELD, TYPE_READ_ONLY_RECORD_CONSTRUCTOR,
      TYPE_RECOVER_TYPE_MISMATCH, TYPE_UNKNOWN_VALUE, UNREACHABLE_AFTER_EXIT);
      every shape emission sits under a comment naming the erased fact.
      **`ir::verify` emits 93 rules** plus its 2 package-path structural
      guards. **Zero source rules anywhere else**: `src/syntaxcheck/` is gone
      and `rg syntaxcheck src/` is empty. The plan-20-Z relocation era — one
      checker per rule, a register of who owns what — is closed: there is no
      register, because there is nothing left to register.)

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

- **C-one-commit (2026-08-29, Phase 1).** Rows 3, 4, 22 (bare form), 33, 38,
  39 (count forms) and 46 landed in ONE commit together with E's
  C-override-typing seam fixup, not one commit per rule as the phase box
  asks. Why: the group was ported into the same working tree while E's gate
  failure (two false TYPE_UNKNOWN_VALUE cascades on rt-* fixtures) was being
  root-caused, and its fixes touch the same `ir::shape` walker; splitting
  the hunks afterwards would have produced commits that do not build.
  The commit message itemizes every rule with its fixture classification.
  Later rows go back to one commit each.
- **C-41-moot (2026-08-29, Phase 1).** TYPE_SUB_CANNOT_RETURN_VALUE is
  unreachable from source: `SUB tick() AS Integer` does not parse
  (`mfb build` on `/tmp/p107-src-subret.mfb` → `MFB_PARSE_EXPECTED_EXPRESSION`
  + `MFB_PARSE_UNEXPECTED_STATEMENT`, exit 1 — the parser reads a return type
  only for a FUNC). No fixture can carry it, so no emission was ported and the
  rule is not in the RELOCATED list; syntaxcheck's site (itself annotated
  "unreachable from source") is deleted with the module. The rule row stays in
  `rules/table.rs` + the spec's code table as a reserved code.
- **C-handler-truncation (2026-08-29, Phase 1 — row 46).** The residue table
  said verify keeps the loop-exit forms of UNREACHABLE_AFTER_EXIT. Inside an
  inline-TRAP handler it cannot: `lower::treeify_handler` drops every
  statement after a terminator before lowering (`statement_terminates` covers
  EXIT/CONTINUE), so `EXIT FOR` + trailing statements in a handler reach the
  IR as the EXIT alone (the pre-D binary reported lines 10–11 of
  `/tmp/p107-src-unreach.mfb`; the port without this fix accepted it). Shape
  owns every exit form while `handler_depth > 0`
  (`ir::shape::tests::loop_exit_tail_inside_a_handler_is_shapes`); verify's
  per-op form keeps the rest.
- **C-recover-literal (2026-08-29, Phase 1 — row 39).** verify's value-type
  form of TYPE_RECOVER_TYPE_MISMATCH was blind to lowering's literal
  coercion: `RECOVER 300` / `RECOVER 0x100` / `RECOVER 1.5` into a `Byte`
  success type lower to `Const Byte "300"`/`"256"`/`"1.5"` (no range check),
  which `compatible(Byte, Byte)` accepts — with syntaxcheck's site deleted the
  new pipeline COMPILED the out-of-range value and crashed codegen on `1.5`
  ("invalid immediate"). The syntaxcheck form was the only thing catching it.
  Fixed in verify (not shape — the evidence survives in the const's text): a
  `$trap_val` assign of a `Const Byte` whose text is not a `u8` is the
  mismatch, with the literal's class (`numeric::classify_literal`) as the
  actual type, exactly the old wording
  (`verify::tests::rejects_recover_literal_lowered_into_a_byte_slot_out_of_range`,
  `syntaxcheck::types::types_tests::byte_recover_rejects_out_of_range_radix_literal`
  through the pipeline oracle).
- **C-package-invalid-home (2026-08-29, Phase 1 — row 20).** The plan sent
  PACKAGE_INVALID's metadata validation to "the decode boundary
  (`cli/build/packages.rs` / `manifest::package`)". It landed in the shape
  pass's import walk instead. Why: the semantic half (an exported type or
  function signature referencing an undeclared type; a non-comparable map
  key) needs the full type table — the project's own declarations plus every
  imported package's — and the resource registry, exactly what
  `Walker.types` / `is_comparable` / `is_resource_type` already are; a copy
  at the decode boundary would have been the third `is_comparable`. The
  decode boundary keeps what it had: `verify_and_report_packages` rejects an
  unreadable container before any checker runs (measured with
  `/tmp/p107-probe-pkg.sh`: `[Tampered]` + PACKAGE_INVALID, exit 1). The
  unreadable-TABLE forms (a well-formed container whose type/resource/export
  table fails to decode) had only the checker, so they moved with the walk.
  One deliberate difference: the checker validated each package's types
  right after installing that package's exports, so a type reaching a
  package imported LATER in source order was "unknown" at that moment; the
  pass validates against the complete table. `read_package_type_exports`
  already resolves foreign types into each package's own export list, so
  the only program that can tell is one whose package metadata references a
  type its own tables do not carry — a malformed package either way.
- **C-structural-filter (2026-08-29, Phase 2).** Deleting the
  `RELOCATED_TO_IR_VERIFY` filter from `collect_source_diagnostics` let
  verify's two package-path STRUCTURAL rules
  (`PACKAGE_BINARY_REPRESENTATION_VERIFY_{TYPE,MATCH}`, not in the source rule
  table) reach the source stream: `syntax/match/control-flow-match-pattern-
  invalid` gained a `0-000-0000 UNKNOWN_RULE` line ("`Rect` is not a variant
  of union `Shape`") beside its TYPE_MATCH_PATTERN_MISMATCH — the list had been
  filtering them implicitly. The source stream now filters exactly those two
  by name (a source program's equivalent defect is reported by its source
  rule); everything else verify holds is its own to emit on both paths.
- **C-stateful-resource (2026-08-29, Phase 2 — row 20 follow-up).** The
  Phase-1 gate (not the corpus) caught a false PACKAGE_INVALID from the new
  package walk on `rt-behavior/native/libsnd-load-sound-rt` and
  `native-resource-state-import-rt`: an imported signature spells a stateful
  resource `SoundFile STATE SoundInfo`, and `is_resource_type` compared the
  whole spelling against the bare table name. It now keys on
  `base_resource_name` (the STATE clause stripped), as every other resource
  lookup does. Lesson recorded: `diag-set-diff.sh` runs only the fixtures
  whose golden already carries a diagnostic, so a NEW error on a clean
  fixture is invisible to it — the gate / test-accept are the only sweeps
  that see one; a relocation that touches package or resource typing must
  run the gate before its landing, not at the letter's end.
- **C-merge-advisory (2026-08-29, finish).** `main` advanced while D ran
  (plan-109 landed) and its `5f17afd7c`/`f27f3f343` added ONE new rule to the
  source checker this letter deletes: plan-109-A's enum-value advisory
  (`EnumVariant::advisory` → `CRYPTO_SHA1_INSECURE`, warn once per
  user-authored `Hash.SHA1`, injected builtin source exempt). Merging `main`
  into `worktree-P-107` therefore hit delete/modify conflicts on three
  checker files; the module stays deleted and the rule is relocated into
  `ir::verify`'s enum member-access arm (`check_enum_member_advisory`): the
  evidence survives lowering (an `Enum.Member` access, in an expression or a
  `MATCH` literal, is one `MemberAccess` value), the injected-source
  exemption keys on the `builtins/<pkg>.mfb` file the IR still records, and
  the rule is source-path only (a decoded package was never source-checked).
  main's two checker tests moved to `ir::tests` over a restored pipeline
  oracle (`testutil::check_src` / `accepts` — deleted in Phase 2 for lack of
  consumers, and back now that it has two).
- **C-table-call-verdict (2026-08-29, finish).** main's new fixture
  `syntax/crypto/hash-removed-spellings-invalid` (a renamed enum member used
  as a builtin-call argument, `crypto::hash(Hash.SHA224, s)`) showed one
  cascade the E-era census had no fixture for: the checker's package-table
  arm typed a MATCHED call by the overload's declared return type even when
  an argument's own type was `Unknown` (`Unknown` is compatible with the
  `Hash` slot), so the binding did not cascade TYPE_UNKNOWN_VALUE; lowering's
  exact registry resolution answers `Unknown` for the same call and the
  shape pass's fallback cascaded (4 extra diagnostics). `checker_types_unknown`
  now answers "typed" for a table-checked builtin call whose verdict was not
  Unknown (`builtins::table_checked_call`); corpus 529 same / 0 set-diff on
  the merged tree (`ir::shape::tests::matched_table_builtin_call_with_an_
  unknown_argument_is_typed`).
- **C-case-line (2026-08-29, finish).** With the enum advisory in verify, the
  `MATCH`-literal occurrence (`CASE Hash.SHA1`) reported at the `MATCH`
  statement's line: verify's Match arm checked pattern values under the op's
  line. The arm now sets the case arm's own `loc` around the pattern walk
  (`check_match_patterns` already did for its rules), so the advisory reports
  at the CASE line as the checker did (`crypto-sha1-advisory-valid` SAME
  again). The other acceptance mismatch was a pure stream reorder of the
  same records (`hash-removed-spellings-invalid`: the checker warned during
  inference, before the call's mismatch; verify's stream follows shape's) —
  golden regenerated, movecheck MOVE-ONLY.
- **C-harness-test-verb (2026-08-29, Phase 1).** `scripts/diag-set-diff.sh`
  re-ran `.testrun` goldens as `mfb test -q …`, but `mfb test` has no `-q`
  (usage error, exit 2) — the harness reported `testing-assert-invalid` as a
  SETDIFF with an empty actual set. The `-q` is now build-only. Any earlier
  "SAME" over a `.testrun` golden predates the `.testrun` support and is
  unaffected.
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
