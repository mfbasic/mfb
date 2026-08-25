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
| plan-106-C complete | `rg -n 'enum Type' src/syntaxcheck/` → 0 | **MET** (2026-08-24, commit `e9eaac094`) — the single hit is the doc comment recording the removal |

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
- ~~The PRE-monomorph passes (`resolve_project` on the source AST, DOC
  validation) stay AST-domain — they run before elaboration by design.~~
  **Corrected in Phase 1 (Correction 1)**: they cannot, because
  `resolve_augmented` IS the pre-pass — both passes funnel through it. The
  pre-pass gains one forward `elaborate()` call instead. No pass is duplicated,
  and no backward edge is created.

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
| syntaxcheck walk surface (AST→HIR port) | **164** walk-arm lines over 8 files | `rg -c 'Statement::\|Expression::\|Item::' src/syntaxcheck/` at kickoff: helpers 42, inference 28, mod 27, checking 21, link 18, builtins 13, types 8, resources 1 |
| resolve_augmented walk surface | **77** walk-arm lines over 2 files | `rg -c 'Statement::\|Expression::\|Item::' src/resolver/` at kickoff: resolution.rs 46, mod.rs 31 |
| `resolve_type_name` recursion sites (the resolver's string-grammar machine) | **30** | `rg -n 'resolve_type_name\(' src/resolver/ \| grep -v 'fn resolve_type_name' \| wc -l` → 30 |

### Verified properties

- **HIR mirrors the AST 1:1** with identical variant names — the port recipe
  (perl word-boundary renames + type-field reads becoming structural) is
  proven four times over (monomorph D3, `resource_escape`, `expand_expect`,
  ir::lower C3). VERIFIED (landed).
- **What `resolve_augmented` does post-monomorph.** VERIFIED (Phase 1, by
  reading `resolver/resolution.rs` and censusing every `self.report` call).
  `resolve_augmented` is not a post-monomorph-only entry point: it is the
  SHARED core both passes funnel through (`resolve_project` →
  `resolve_project_with` → `augment_project` → `resolve_augmented`). It runs
  exactly two things — `resolve()` (always) and `resolve_doc_blocks()` (only
  when `validate_docs`, i.e. only the pre-pass). Its checks split as:

  | Kind | Rules | Disposition |
  |---|---|---|
  | Name resolution over the walk | `SYMBOL_UNKNOWN_IDENTIFIER` ×5, `SYMBOL_DUPLICATE_LOCAL` ×4, `SYMBOL_DUPLICATE_IMPORT` ×4, `SYMBOL_UNKNOWN_IMPORT`, `SYMBOL_DUPLICATE_TOP_LEVEL` ×6, `SYMBOL_RESERVED_BUILTIN_NAME`, `TYPE_DUPLICATE_FIELD`/`_VARIANT`/`_ENUM_MEMBER` | **Port mechanically.** Pure structural walk; no type strings. |
  | Type-string work | `SYMBOL_UNKNOWN_TYPE` ×2 + `TYPE_RESULT_NOT_USER_VISIBLE`, all reached through `resolve_type_name(&str)` | **NOT already gone post-105-B** — the plan's guess was wrong. `resolve_type_name` is still a 120-line string-grammar machine: `strip_type_group`, `== "Result"` / `starts_with("Result OF ")`, `strip_prefix("ISOLATED FUNC(")` / `("FUNC(")` + `split_func_params_and_return`, `thread_parts_full`, a `parse(…)` whose four container arms immediately **re-render** each child with `.name()` to recurse, `state_type_name`/`base_resource_name`, a second `parse(…)` for `UserOf` that re-renders its args, `== "Unknown"`, and `contains('.')`. It is the single largest no-strings violation left outside codegen, and it becomes `resolve_type(&ParameterType)`. |
  | Resource-decl / LINK checks | `RESOURCE_CLOSE_NOT_NATIVE` ×2, `RESOURCE_CLOSE_SIGNATURE`, `RESOURCE_CLOSE_MISSING` | Port mechanically (`HirItem::Resource`/`Link` reuse the AST node verbatim). |
  | DOC validation (18 `DOC_*` rules) | `resolve_doc_blocks` | **Pre-pass only** (`validate_docs`). Reads `HirItem::Doc`, whose `DocBlock` is the verbatim AST node. |

  Nothing here is redundant-with-the-pre-pass in a way this letter may delete:
  the post-pass sees the *monomorphized* program, so it resolves names the
  pre-pass never saw (instantiated generics, mangled overloads). The
  dual-resolve observation stands as separate work, as §Non-goals says.

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

- [x] Read `resolve_augmented` post-monomorph responsibilities; record the
      inventory in §2 (replacing UNVERIFIED). Two findings changed the shape of
      this phase and are in §Corrections: `resolve_augmented` is the SHARED core
      of both resolve passes (not a post-monomorph-only entry point), and its
      `resolve_type_name` is still a full string-grammar machine.
- [x] Port `validate_entry_point` to `&HirProject` (both callers, plus the
      `manifest/mod.rs` integration test). Both type facts it compared as string
      literals are now `ParameterType` values: `returns != "Integer"` →
      `returns != ParameterType::Integer`, and
      `param.type_name.as_deref() == Some("List OF String")` →
      `param.type_ == ParameterType::list_of(ParameterType::String)`. The
      `ir::EntryPoint.returns` field was already a `ParameterType`, so the
      render-then-`parse` at the return was deleted outright rather than moved.
- [x] **Prerequisite discovered mid-phase (Correction 3): `ParameterType::parse`
      does not peel a grouped type name.** Added as a task rather than deferred,
      per §Do-the-work — the resolver port cannot proceed without it.
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

### Correction 1 — `resolve_augmented` is the SHARED core, not a post-monomorph entry point

The plan (§Non-goals) assumed `resolve_project` and `resolve_augmented` were
separable, so the pre-monomorph pass could stay AST-domain while the
post-monomorph one moved to HIR. Measured:

```
$ rg -n 'fn resolve_project|fn resolve_project_with|fn resolve_augmented|fn validate_project_docs' src/resolver/mod.rs
46:pub fn resolve_project(          -> resolve_project_with(.., true)
68:pub fn resolve_project_with(     -> augment_project(ast)? -> resolve_augmented(..)
83:pub fn resolve_augmented(        -> Resolver::new(..).resolve()
58:pub fn validate_project_docs(    -> Resolver::new(..).resolve_doc_blocks()
```

Every resolve in the compiler goes through one `Resolver`. Making
`resolve_augmented` take `&HirProject` therefore moves ALL of them.

Keeping the pre-pass AST-domain would require a second copy of the 2,375-line
`resolution.rs` walk — the exact duplication this plan exists to remove, and a
guaranteed future divergence. So the resolver becomes HIR-only and the three
AST-domain entry points (`resolve_project_with`, `validate_project_docs`, and
`cli/doc.rs`'s single-file path) each gain one forward `crate::hir::elaborate`
call.

This is not a backward edge and does not violate the terminal invariant:
`elaborate` is AST→HIR, the same direction the compile path already runs. The
cost is one structural walk on a project already being walked several times.
The §Non-goals bullet is struck through above.

### Correction 3 — `ParameterType::parse` did not peel a grouped type name

Found while designing `resolve_type`. Probed directly:

```
PROBE "(List OF String)" = UserOf("(List", [Named("String)")])
PROBE "List OF String"   = ListOf(String)
```

A grouped spelling (`LET y AS (Integer)`, `List OF (Map OF String TO Integer)`,
`Thread OF (List OF String) TO String` — 4 fixtures use them, and bug-105 exists
precisely to keep them working) parsed into *garbage*. It survived only because
`name()` echoed the garbage back verbatim, and because every consumer called
`strip_type_group` at its own position first: `resolver::resolve_type_name:1278`,
`syntaxcheck::parse_type:39`, `monomorph::lower:1647` and `:1750`.

That is exactly the failure mode this plan exists to end. A consumer that walks
variants instead of re-parsing a string has nowhere left to strip, so
`resolve_type(&ParameterType)` would have hit `Named("(Integer)")`, missed
`self.types`, and re-opened bug-105 as `SYMBOL_UNKNOWN_TYPE`.

Fixed at the grammar: `parse` peels a whole-name group before anything else, and
therefore at every level of the recursion. `List OF (Map OF String TO Integer)`
now yields `ListOf(MapOf(String, Integer))`. The depth check in
`strip_type_group` keeps `(A) TO (B)` intact, and a `FUNC(…)` type never starts
with `(`, so neither is touched.

This is the one deliberate exception to `parse`↔`name` byte-exactness
(`[[hir-parse-name-roundtrip-load-bearing]]`): `parse("(Integer)").name()` is
`"Integer"`. It is safe because the normalized form is precisely what all four
consumers above already computed. **Proven, not argued** — the group-peel plus
the entry port together produce:

```
artifact-gate.sh target/release/mfb all -> 1255 tests, 1402 build(s),
                                           1730 golden(s) checked, 0 diff(s)
test-accept.sh                          -> acceptance tests passed (1271 ran)
cargo test --bin mfb                    -> 3650 passed, 0 failed
```

Zero golden movement across every fixture that spells a grouped type. The four
`strip_type_group` call sites are now redundant and are removed as their
consumers port.

### Correction 2 — `resolve_type_name` is still a full string-grammar machine

§2 predicted the resolver's type-string work "should already be gone
post-105-B". It is not. `resolver/resolution.rs:1265-1392` is 120 lines of
grammar re-implementation with **30** recursion sites, and its four
`ParameterType::parse` container arms *re-render every child with `.name()`*
just to recurse by string. Measured before the port:

```
$ rg -n 'strip_prefix|starts_with\("|thread_parts_full|state_type_name|base_resource_name|strip_type_group|split_func_params' src/resolver/resolution.rs
1278:  strip_type_group        1288:  starts_with("Result OF ")
1298:  strip_prefix("ISOLATED FUNC(")   1302:  strip_prefix("FUNC(")
1306:  thread_parts_full       1353:  state_type_name   1354:  base_resource_name
1407:  split_func_params_and_return
```

Phase 1 therefore does more than a node-type rename: `resolve_type_name(&str)`
becomes `resolve_type(&ParameterType)`, a closed match on the variants, and all
eight grammar helpers above leave the resolver. Recorded here rather than
deferred, per §Do-the-work.

## Summary

The last backward edge dies, and with it the `parse↔name` load-bearing seam.
Risk is walk breadth, not design — the port recipe has landed four times. The
review's dual-resolve/diagnostic-streaming observations are recorded as
separate work, not braided in.
