# plan-102-D: Lift elaborate above monomorph (generic AST → generic HIR)

Last updated: 2026-08-23
Effort: x-large (1d–3d) — re-measure and split into parts at kickoff
Depends on: plan-102-C (HIR exists and `ir::lower` consumes it; `elaborate` works
on concrete code).

Move `elaborate` above monomorph so it lowers the **generic** AST to **generic**
HIR — which means classifying type variables (`Var`) and relocating overload
resolution into `elaborate`, and porting monomorph to consume/produce HIR. To keep
this landable, monomorph is ported *mechanically* first: it consumes generic HIR
and produces concrete HIR, but its `unify`/`substitute` still run the existing
string algorithm internally by rendering `ParameterType` via `name()`. Swapping
those internals to native typed operations is plan-102-E. After this sub-plan:
`… AST → elaborate → generic HIR → monomorph (HIR→HIR, string-algorithm inside) →
concrete HIR → ir::lower → IR`.

See plan-102-A §3 for the full layering, the byte-identity gate, and the roadmap.

References:

- `src/monomorph/mod.rs` (driver, overload state), `src/monomorph/lower.rs` (the
  AST walk), `src/monomorph/helpers.rs` (`unify_type` `:41`, `substitute_type_params`
  `:171`).
- `src/ast/types.rs` — `template_params` on `Function` (`:530`) / `TypeDecl`
  (`:489`): the generic-parameter lists `elaborate` uses to classify `Var`.
- `src/types.rs` — `ParameterType::Var` (interned post-A).
- `src/hir/`, `src/ir/lower.rs` — from plan-102-C.

## Prerequisites

See plan-102-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-102-C complete | `ir::lower` consumes HIR; `elaborate` covers the corpus | NOT MET until C lands |

## 1. Goal

- `elaborate` runs on the generic AST (before monomorph) and produces generic HIR:
  it classifies each type-name leaf as `ParameterType::Var` (when the name is in the
  enclosing decl's `template_params`) or `Named` (otherwise), and it resolves
  overloads (picks a signature per call by argument types).
- monomorph consumes generic HIR and produces concrete HIR (generics instantiated,
  names mangled), feeding the HIR→IR lowering from plan-102-C. Its internal
  `unify`/`substitute` still use the string algorithm (via `name()`) — a
  transitional shim removed in plan-102-E.

### Non-goals (explicit constraints)

- No change to compiled output — byte-identical `.ncode`/`.ncodesum`.
- Do not yet replace monomorph's string `unify`/`substitute` internals with typed
  ones — that is plan-102-E. This sub-plan is the *structural* move (elaborate
  above, monomorph on HIR); the algorithm swap is E.
- No change to overload-selection results, generic-instantiation results, or
  mangled names (byte-identity guards this).

## 2. Current State

monomorph runs on the string AST between the two resolve passes
(`src/cli/build/mod.rs:332`) and does four things (plan-102-A §Current State of the
feature): instantiate generics, unify/substitute (string), resolve overloads, infer
expression types. It decides variable-ness by **param-list membership**
(`src/monomorph/helpers.rs:47` — `params.iter().any(|param| param == pattern)`), not
by a `Var` variant — verified. That means `elaborate` can classify `Var` from the
same `template_params` lists the AST already carries (`src/ast/types.rs:489,530`)
without new information.

### Measured populations

| What | Count | Command |
|---|---|---|
| monomorph `unify_type`/`substitute_type_params` call sites | 56 | `rg -c 'unify_type\|substitute_type_params' src/monomorph/ \| awk -F: '{s+=$2} END{print s}'` |
| monomorph `HashMap<String, String>` (substitution/type maps) | 21 | `rg -c 'HashMap<String, String>' src/monomorph/ \| awk -F: '{s+=$2} END{print s}'` |
| monomorph `.to_string()` (string type churn) | 103 | `rg -c '\.to_string\(\)' src/monomorph/ \| awk -F: '{s+=$2} END{print s}'` |
| monomorph size | 4136 | `find src/monomorph -name '*.rs' \| xargs wc -l \| tail -1` |
| overload-resolution helpers to relocate | UNMEASURED | measure at kickoff: `rg -n 'overload' src/monomorph/` |

### Verified properties

- **Variable-ness = param-list membership, not a `Var` variant.**
  `src/monomorph/helpers.rs:47`. So `elaborate` classifying `Var` from
  `template_params` is faithful to monomorph's own rule. VERIFIED.
- **`template_params` are on the decl in the AST.** `src/ast/types.rs:489` (TypeDecl),
  `:530` (Function). VERIFIED.

## 3. Design Overview

- **`elaborate` on generic code:** walk with the enclosing decl's `template_params`
  in scope; a type-name leaf whose symbol is in that set → `ParameterType::Var`,
  else `Named`. This is a local, scope-aware decision needing no cross-module
  resolution (plan-102-A §Current State).
- **Relocate overload resolution into `elaborate`:** picking which signature a call
  selects needs argument types, which `elaborate` now has. It emits HIR calls
  resolved-to-a-signature but still generic. (Measure the monomorph overload code
  first; it moves, it is not duplicated.)
- **Port monomorph to HIR (mechanical):** monomorph consumes generic HIR and
  produces concrete HIR. Keep its `unify`/`substitute` string algorithm by
  rendering `ParameterType` via `name()` at the boundary of those two functions
  only — a small, localized shim so the *structural* move lands byte-identically
  before the *algorithm* swap (E).

**Coupling, called out:** "elaborate above monomorph" and "monomorph consumes HIR"
are one atomic change — monomorph's input becomes HIR exactly when elaborate moves
above it. They cannot land in separate sub-plans. That is why this sub-plan is
x-large and why the *algorithm* swap is deferred to E (the one seam that *can* be
cut cleanly). Split this sub-plan at kickoff along scope, e.g. D1 = `Var`
classification in `elaborate` + generic-HIR shape; D2 = overload-resolution
relocation; D3 = monomorph HIR port with the `name()` shim.

### Rejected alternatives

- **A pipeline-level HIR→AST bridge feeding the unchanged string monomorph.**
  Rejected: it double-elaborates and adds a throwaway renderer to the *pipeline*.
  The `name()` shim *inside* monomorph's two functions is smaller and local.
- **Land the algorithm swap in the same step (skip E).** Rejected: bundles the
  structural move with the algorithm rewrite, making an already-x-large step
  un-reviewable; E is a clean seam.

## Compatibility / Format Impact

None externally observable.

## Phases

> Re-measure and split into D1/D2/D3 at kickoff.

### Phase 1 — `Var` classification in `elaborate`; generic-HIR shape

- [x] `elaborate` walks with `template_params` in scope and emits `Var`/`Named`
      leaves; generic HIR is representable end to end. (Added
      `ParameterType::with_vars(&[String])` — recursively reclassifies `Named` leaves
      matching the template params to `Var`; `elaborate`'s parse helpers apply it and
      the `template_params` are threaded through the whole elaborate chain — function/
      type decl provide their `template_params`, top-level bindings use `&[]`.)
- [x] Tests: a generic decl elaborates to HIR whose `Var` leaves match the decl's
      `template_params`. (`generic_decls_classify_type_variables_as_var`: a generic
      FUNC `first OF T (xs AS List OF T, i AS Integer) AS T` → `List OF Var`, `Integer`
      scalar, `Var` return; a generic TYPE `Box OF E` → `Var` field vs `Integer`.)

Acceptance: generic elaboration unit tests pass; `cargo test` green (elaborate not
yet on the generic path in the build). VERIFIED — 6/6 `hir::` tests pass, 3625 bin
unit tests pass; **byte-identical** (monomorph clears `template_params` on every
instantiated decl → `with_vars` is a no-op on the concrete post-monomorph input the
build still feeds `elaborate`; gate `diff` vs baseline IDENTICAL).
Commit: —

### Phase 2 — relocate overload resolution into `elaborate`

- [ ] Move monomorph's overload-selection logic into `elaborate` (measure it first);
      `elaborate` emits signature-resolved HIR calls.
- [ ] Tests: overload-selection cases (the datetime/net overload fixtures) resolve
      identically.

Acceptance: overload fixtures byte-identical; `cargo test` green.
Commit: —

### Phase 3 — port monomorph to HIR (string algorithm via `name()` shim)

- [ ] monomorph consumes generic HIR, produces concrete HIR; `unify`/`substitute`
      keep the string algorithm behind a `name()` shim at their boundary. Wire
      `elaborate` above monomorph in the build (`src/cli/build/mod.rs`).
- [ ] Tests: full suite.

Acceptance: `artifact-gate all` no NEW diff vs the plan-102-A baseline; `cargo
test` green; `test-accept` no NEW mismatch.
Commit: —

## Validation Plan

- Tests: generic-elaboration + overload-selection units; full IR/codegen/
  `rt-behavior` suite; the generics/monomorph fixture set specifically.
- Coverage check: every generic/overloaded fixture flows through `elaborate` +
  monomorph after Phase 3.
- Runtime proof: `artifact-gate all` byte-identical (modulo baseline) — proves
  instantiation and overload results are unchanged.
- Doc sync: update the spec/`.ai` HIR chapter to note elaborate now handles generics
  + overloads; note monomorph now runs on HIR.
- Acceptance: `cargo test`; `artifact-gate all`; `test-accept`; fmt both crates.

## Open Decisions

- **Split axis for D1/D2/D3.** Recommend the three above (Var / overloads / port).
  Re-measure the overload code and the monomorph AST-walk surface at kickoff to
  confirm each part is one sitting. (§3)

## Corrections

<Filled in during execution.>

## Summary

The structural pivot of the feature: semantic analysis (name resolution, typing,
`Var` classification, overload resolution) consolidates into `elaborate`, and
monomorph moves onto HIR. It is deliberately split from the *algorithm* swap (E) so
the risky structural move lands byte-identically with monomorph's proven string
logic still running underneath.
