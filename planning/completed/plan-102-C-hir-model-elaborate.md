# plan-102-C: HIR data model + elaborate(concrete AST → HIR)

Last updated: 2026-08-23
Effort: x-large (1d–3d) — re-measure and split into parts at kickoff
Depends on: plan-102-B (the IR must already be `ParameterType`-typed; HIR lowers
into that typed IR).

Introduce the HIR: a typed, name-resolved, tree-shaped layer between the AST and
the IR. Build the `elaborate` pass (AST → HIR) and stand it up first on the
**concrete, post-monomorph AST** — the easy case, where no `Var` and no generics
exist, so elaboration only has to resolve names and attach `ParameterType`s. Then
switch `ir::lower` to consume HIR (HIR → IR) instead of the AST. After this
sub-plan the pipeline is `… → monomorph (string AST) → concrete AST → elaborate →
concrete HIR → ir::lower → IR`. Lifting `elaborate` above monomorph, and moving
monomorph itself onto HIR, are plan-102-D/E.

See plan-102-A §3 for the full layering, the byte-identity gate, and the roadmap.
This is the design-uncertainty concentration of the whole feature: standing HIR up
on concrete code first is the cheapest experiment that falsifies the HIR node
design before the harder generic case (D).

References:

- `src/ast/types.rs` — the AST node shapes HIR mirrors; note `state_type`/
  `return_state_type` and `resource`/`return_resource` are already **separate**
  fields (not baked into the type string), so HIR type fields carry the bare type.
- `src/ir/lower.rs` — becomes HIR → IR (currently AST → IR).
- `src/types.rs` — `ParameterType` (post-A: interned, complete).
- `.ai/testing-gates.md`, `.ai/codegen-invariants.md`.

## Prerequisites

See plan-102-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-102-B complete | IR type fields are `ParameterType` (`rg 'type_: String' src/ir/value.rs` → 0) | NOT MET until B lands |

## 1. Goal

- A HIR module (`src/hir/`) defining typed, name-resolved, tree-shaped nodes that
  mirror the AST's structure but whose type fields are `ParameterType` (generic-
  capable: `Var` is representable, though none occur on concrete input yet).
- An `elaborate` pass that lowers the post-monomorph concrete AST to HIR: resolves
  names and attaches the `ParameterType` for every typed node (via
  `ParameterType::parse`, since the input is concrete).
- `ir::lower` consumes HIR (HIR → IR); the AST → IR path is retired.

### Non-goals (explicit constraints)

- No change to compiled output — byte-identical `.ncode`/`.ncodesum`.
- Do not move `elaborate` above monomorph and do not touch monomorph internals
  (that is D/E). `elaborate` runs on the *concrete* AST in this sub-plan.
- Do not yet relocate overload resolution or type checking into `elaborate` — those
  still live in monomorph/resolver/syntaxcheck for now (D/F). `elaborate` here only
  does name resolution + type attachment on already-resolved concrete code.

## 2. Current State

`ir::lower_augmented_project` consumes the post-monomorph concrete AST directly
(`src/ir/lower.rs:140-141`). There is no intermediate typed layer; type attachment
and any residual name work happen inline during lowering. The AST is string-typed
and stays so (plan-102-A §3 rejected alternatives). The AST already separates the
`RES`/`STATE` axes from the type string (verified: `src/ast/types.rs` —
`state_type: Option<String>` `:477`, `return_state_type` `:536`, `resource: bool`
`:475`, `return_resource: bool` `:534`), so HIR's typed fields carry the bare type
with `RES`/`STATE` as sibling fields, exactly as the AST does.

### Measured populations

| What | Count | Command |
|---|---|---|
| `ir::lower_augmented_project` call sites (all become HIR-fed) | 5 | `rg -n 'lower_augmented_project' src/cli/build/mod.rs \| wc -l` |
| AST node structs/enums (the shapes HIR must mirror) | 51 | `rg -c 'pub (struct\|enum) ' src/ast/types.rs` (plus `expr.rs`) |
| `ir/lower.rs` size (rewrites its front half to read HIR) | 3825 | `wc -l src/ir/lower.rs` |

Re-measure at kickoff: the exact count of AST type-bearing fields HIR must carry
(the `type_name`/`return_type`/`element_type`/… sites across `ast/types.rs`,
`ast/expr.rs`, `ast/stmt.rs`) — this sets the C split.

### Verified properties

- **The AST separates `RES`/`STATE` from the type string.** Read
  `src/ast/types.rs:471-572`. So HIR type fields are the bare `ParameterType`, with
  `resource`/`state_type` as separate fields — no need for `ParameterType` to model
  `STATE`. VERIFIED.
- **`elaborate` on concrete input never produces `Var`.** By construction the input
  is post-monomorph (generics erased). So this sub-plan exercises the HIR/`elaborate`
  machinery without the hard `Var`-classification problem — which is deferred to D.
  VERIFIED by the pipeline order (`monomorph` precedes `ir::lower`,
  `src/cli/build/mod.rs:332` before `:416`).

## 3. Design Overview

- **HIR nodes** (`src/hir/`): one typed node per AST node kind, same tree shape,
  type fields `ParameterType`. Mirror the AST's `RES`/`STATE` sibling-field layout.
  Keep source spans/lines for diagnostics.
- **`elaborate(&AstProject) -> HirProject`**: a structural walk that resolves names
  and attaches `ParameterType`s. On concrete input, type attachment is
  `ParameterType::parse` of the AST's type string; name resolution mirrors whatever
  `ir::lower` does inline today (extract it into `elaborate`).
- **`ir::lower` becomes HIR → IR**: its front half (which read AST + strings) now
  reads HIR + `ParameterType`; its back half (IR construction) is unchanged since
  the IR is already typed (plan-102-B).

**Split at kickoff.** Likely parts: C1 = HIR node module + `elaborate` skeleton
producing HIR for a subset (decls only), validated by a HIR→AST render round-trip;
C2 = full `elaborate` (statements + expressions); C3 = switch `ir::lower` to
HIR→IR and delete the AST→IR path.

### Rejected alternatives

- **Skip HIR; type the AST.** Rejected in plan-102-A §3 (parser can't classify
  `Var`; AST must hold invalid spellings).
- **Make `elaborate` also resolve overloads/type-check now.** Rejected here to keep
  C's uncertainty bounded to "is the HIR node shape right on concrete code"; the
  overload/type-check relocation is D/F.

## Compatibility / Format Impact

None externally observable. HIR is internal; IR/wire format unchanged.

## Phases

> Re-measure and split into C1/C2/C3 at kickoff.

**Kickoff design decisions (2026-08-23), from studying `ast/types.rs` + `ir/lower.rs`:**
- **`elaborate` carries DECLARED types only; expression-type INFERENCE stays in
  `ir::lower`.** AST expressions carry no result type — `ir::lower` INFERS it
  (`expression_type` `src/ir/lower.rs:1931`, plus the String maps
  `function_returns`:1749/`function_types`:1800/`function_params`:1870/
  `declared_binding_types`:1896) and stamps it onto `IrValue.type_`. So `elaborate`
  is a structural copy that attaches only the DECLARED type annotations (param/return/
  binding/field types, `SetLiteral.element_type`, `MapLiteral.key/value_type`,
  `Constructor.type_name`, `MatchPattern::Union.type_name`) via `ParameterType::parse`
  (concrete input → concrete type, no `Var`). The inference machinery is NOT moved
  into `elaborate` in C — it stays in `ir::lower` and, in C3, reads HIR instead of the
  AST. (Moving overload resolution/inference up is plan-102-D, not C.)
- **RES/STATE stay sibling fields** (`resource: bool`, `state_type: Option<ParameterType>`),
  mirroring the AST — HIR type fields carry the BARE `ParameterType` (verified: AST
  separates them at `types.rs:475/477,534/536,567/569`).
- **Absent annotation → `parse("Unknown")`**, matching `ir::lower`'s
  `.unwrap_or_else(|| "Unknown".to_string())` in `lower_param`/`lower_binding`.
- **Native/link/doc/testing `Item`s reuse the AST structs** in HIR (`HirItem::Resource(
  ast::ResourceDecl)` …) — they carry no source type strings needing retyping on
  concrete code; the ParameterType retype focuses on Binding/Function/Type/Statement/
  Expression.
- **Split executed as: C1+C2 = HIR node module (`src/hir/mod.rs`, full decls +
  statements + expressions) + `elaborate` for the whole AST + round-trip test**
  (landed together to avoid a stub-body HIR); **C3 = rewire `ir::lower` front-half to
  consume HIR** (the byte-identity-critical step). C1+C2 is not wired into the build
  (transient dead_code until C3, same pattern as A-phase-2's interner).

### Phase 1 — HIR node module + `elaborate` skeleton (decls), round-trip validated

Landed together with Phase 2 as one `src/hir/mod.rs` (953 lines, 16 HIR types) to
avoid a stub-body HIR.

- [x] Add `src/hir/` node types mirroring `ast/types.rs` decls, type fields
      `ParameterType`, `RES`/`STATE` as sibling fields. (`HirProject`/`HirFile`/
      `HirItem`/`HirFunction`/`HirTypeDecl`/`HirField`/`HirParam`/`HirBinding`/… —
      type fields `ParameterType`, `resource: bool` + `state_type: Option<ParameterType>`
      siblings; native/doc/testing `Item`s reuse the AST structs; `Copy` AST enums
      reused directly.)
- [x] `elaborate` handles top-level decls (types, functions, bindings) on concrete
      AST via `ParameterType::parse` (absent annotation → `parse("Unknown")`).
- [x] Tests: round-trips a corpus of concrete decls to the same type spellings the
      AST held (`ParameterType::name()` equality). (5 unit tests.)

Acceptance: round-trip test passes; `cargo test` green (nothing wired into the
build path yet). VERIFIED — 5/5 `hir::` tests pass; build + test-compile 0 errors/0
warnings; module unwired.
Commit: 15aed5119 (C1+C2)

### Phase 2 — full `elaborate` (statements + expressions)

- [x] Extend `elaborate` to statements and expressions (all `Statement`/`Expression`
      variants; `SetLiteral.element_type`/`MapLiteral.key/value_type`/`Constructor.type_`/
      `MatchPattern::Union.type_` retyped to `ParameterType`; no silent catch-all —
      exhaustive matches). Corrected the round-trip test's constructor case to bracket
      syntax `Point[1,2]` (parens `Point(1,2)` fresh-parse to `Call`, not `Constructor`).
- [x] Tests: `elaborate` produces a complete HIR for a mixed statement/expression
      corpus without panics; HIR type spellings match the AST's.

Acceptance: `elaborate` covers the corpus; `cargo test` green. VERIFIED (5/5 pass).
Commit: 15aed5119 (C1+C2)

### Phase 3 — switch `ir::lower` to HIR → IR

**C3 approach (kickoff):** a mechanical **input-swap**. `lower_augmented_project`
and its front-half helpers change their AST parameter to the HIR: `ast: &AstProject`
→ `hir: &HirProject`, and every read of an AST type string (`node.type_name`,
`.return_type`, `.element_type`, …) repoints to the HIR node's `ParameterType` field,
rendered back with `.name()` where ir::lower's existing **String** inference
machinery (`expression_type`, `function_returns`/`function_types` maps) needs a
string. Because `ParameterType::parse`→`.name()` round-trips byte-exact (plan-102-A
guarantee), elaborate-then-render is a no-op on the type spellings, so codegen output
is byte-identical. This establishes the HIR→IR boundary (C's real purpose — the
scaffold D builds on) WITHOUT rewriting ir::lower's internal String inference to
native `ParameterType` (that native conversion is deferrable, exactly like the
`ir::verify` internal-representation conversion in plan-102-B Phase 3). The
non-type structural reads (names, bodies, `line`s) repoint to the identical HIR
fields.

- [x] Rewrite `ir::lower`'s front half to read HIR; wire `elaborate` into the build
      after monomorph; delete the AST → IR path. (Done via the input-swap:
      `lower_augmented_project` KEEPS its `&AstProject` signature and elaborates
      INTERNALLY — `let hir = crate::hir::elaborate(ast)` — so the 5 build call sites
      and all test callers are UNCHANGED. The per-node walk + map builders + TypeIndex
      + inference all consume HIR, rendering `.name()` where the internal String
      machinery needs it; native/link/doc `Item`s keep the original AST. The `expect`
      testing desugar bridges through `deelaborate_call_args`/`elaborate_statements`.
      Native LINK function params stay AST (reused structs).)
- [x] Tests: full suite. (3624 bin unit tests pass; all integration binaries pass.)

Acceptance: `artifact-gate all` no NEW diff vs the plan-102-A baseline; `cargo
test` green; `test-accept` no NEW mismatch. **VERIFIED** — gate `diff` vs baseline
IDENTICAL; full suite's sole failure is the recorded `artifact_gate_all` baseline;
production + test build 0 errors/0 warnings.
Commit: d37581ec7

## Validation Plan

- Tests: HIR round-trip (C1/C2), full IR/codegen/`rt-behavior` suite (C3).
- Coverage check: `elaborate` and the HIR→IR path are exercised by every fixture
  (all flow through the build path after C3).
- Runtime proof: `artifact-gate all` byte-identical (modulo baseline).
- Doc sync: add HIR to `.ai/codegen-invariants.md` / spec `architecture/` (the IR
  chapter gains a preceding HIR chapter). Confirm with the spec-sync obligation in
  `.ai/specifications.md`.
- Acceptance: `cargo test`; `artifact-gate all`; `test-accept`; fmt both crates.

## Open Decisions

- **HIR as owned rebuild vs. AST parameterized over the type repr.** Recommend a
  separate owned `src/hir/` module (an owned rebuild), not `Ast<T>` generics —
  parameterizing the whole AST over the type representation is heavy in Rust and
  couples the two layers. (§3)
- **Does `elaborate` subsume the inline name work in `ir::lower`, or call the
  existing resolver?** Recommend extracting the inline work into `elaborate` so
  `ir::lower` becomes a pure HIR→IR structural lowering. (§3)

## Corrections

- **POST-ARCHIVE CORRECTION (2026-08-24): C3 was ticked with backward (HIR→AST)
  seams still inside the lowering path — the goal "the AST → IR path is retired"
  did NOT hold as reported.** `lower_augmented_project` internally de-elaborated
  the whole project to feed the native/link/doc extractors (which needed no AST
  at all — HIR reuses those structs verbatim), `resource_escape::analyze_function`
  was fed a per-function de-elaboration, and the inline `expect(...)` desugar
  round-tripped through AST. Byte-identity could not catch this: a
  `parse↔name`-exact round-trip satisfies the gate while violating the
  stay-typed design. Fixed in commit `6db8e040b` (extractors walk HIR;
  `resource_escape` and `expand_expect` ported to HIR with a new `hir::build`
  module; dead render helpers deleted). The one remaining de-elaboration is the
  post-monomorph validator seam (`cli/build/mod.rs:341`, `audit/mod.rs:111` —
  resolver/entry/syntaxcheck), recorded in plan-102-D's Corrections and retired
  when those validators move onto HIR/`ir::verify`.

## Summary

The design-uncertainty heart of the feature: it fixes the HIR node shape and the
`elaborate` contract. Standing it up on concrete (no-`Var`) code first means the
HIR design is proven byte-identically before the hard generic case (D) is
attempted. If the HIR node shape is wrong, this is where it is cheapest to find out.
