# plan-106-A: Middle-end inference onto ParameterType (ir::lower + monomorph engines)

Last updated: 2026-08-24
Overall Effort: huge (>3d) — the whole plan-106 feature
Effort: large (3h–1d)
Depends on: nothing within 106 (plan-104 AND plan-105 are prerequisites; see gate)

Retype the two middle-end type-inference engines — `ir::lower::expression_type`
and monomorph's `expression_type` — and their `String` type environments onto
`ParameterType`. This begins plan-106, whose end state is the review's
Recommendation #1+#2 finished in full (`planning/Compiler Pipeline.md:67-68`):
**no internal type-string representation, parsing, or comparison anywhere in
the compiler** — the "NO STRINGS" terminal invariant defined below, which every
letter's acceptance builds toward and letter E certifies with a census.

This sub-plan is the **lead document for plan-106**. Roadmap (letter order =
implementation order):

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | `ir::lower` + monomorph inference engines typed; their `String` env maps gone | large |
| **B** | `ir::verify` typed: `infer_type -> ParameterType`, the 42 `String` env-map sites, structural helpers | large |
| **C** | syntaxcheck's private `Type` enum REPLACED by `ParameterType`; its 1,077-line parser reduced to the canonical one | large |
| **D** | syntaxcheck/resolve-augmented/entry-validation consume **HIR**; `deelaborate` (16 fns) DELETED; audit path switched | large |
| **E** | Consolidation (one numeric-promotion source, codegen sibling walks merged) + the terminal no-strings census | medium–large |

## The terminal invariant (what "NO STRINGS" means, checkably)

At the end of plan-106, type information exists as `ParameterType` everywhere
between the source parser and the renderers. The ONLY permitted type-string
sites, each enumerable by grep, are:

1. **Source domain (parse-in #1):** the language grammar in `src/ast/`
   (`ast/expr.rs` tokenizes type syntax into the AST's string fields — the AST
   deliberately stays strings: plan-102-A's settled decision, since the parser
   cannot classify names without scope).
2. **The one parser:** `ParameterType::parse` in `src/types.rs`, called from:
   `hir::elaborate` (the AST→typed boundary), wire decoders
   (`.mfp`/`.ir` binary+JSON decode, external-signature decode), AST-domain
   passes that must query source strings pre-elaboration (resolver — via the
   canonical parser only, per plan-105-B), and tests.
3. **Render-out:** `ParameterType::name()` at serializers (`.ir`/`.nir`/`.mfp`
   emit), diagnostic message formatting, symbol mangling, and doc/man
   signature text.

Everything else is ZERO, certified by letter E's census:
- hand-rolled type-grammar parsing (`strip_prefix("List OF …`-family) outside
  `src/types.rs` + `src/ast/` → 0
- scalar type-string compares (`== "Integer"`-family) → 0
- `format!("List OF …")`-family type construction outside `name()` → 0
- `HashMap<String, String>`-style **type** environments → 0
- HIR→AST de-elaboration → **deleted** (0 functions)
- syntaxcheck's private `Type` enum + parser → **deleted**
- the driver's signature round-trip → already deleted (plan-105-A)

References:

- `planning/Compiler Pipeline.md:27-29,67-68` — the engine census and mandate.
- `src/ir/lower.rs` — `expression_type` (`:1940`, returns `Option<String>`),
  the `String` maps (`function_returns`/`function_types`/`function_params`/
  `declared_binding_types`, `locals: HashMap<String, String>`),
  `numeric_binary_result_type` (`:3645`) + `promote_loop_numeric_type_name`
  (`:1584`) duplicate copies.
- `src/monomorph/mod.rs` — `FunctionContext` (`locals`/`function_returns`/
  `function_types`/`globals`: all `HashMap<String, String>`,
  `enclosing_return: Option<String>`); `src/monomorph/lower.rs` —
  `expression_type` (`:1823`); `src/monomorph/helpers.rs` — the
  `numeric_binary_result_type` (`:594`) + `promote_loop_numeric_type_name`
  (`:598`) copies.
- `src/numeric.rs` — the base numeric algebra (`:378` per the review) that
  becomes the single promotion source (started here, finished in E).

## Prerequisites

Shared by every plan-106 letter; stated once here.

| Must be true | Command | Status |
|---|---|---|
| plan-104 complete (NIR + codegen typed) | plan-104-A..D archived; `rg -c 'type_: String' src/target/shared/nir/mod.rs` → 0 | NOT MET — plan-104 not started |
| plan-105 complete (driver + one grammar) | plan-105-A/B archived; `rg -n 'rsplit_once\(" AS "\)' src/` → 0; `rg -n 'user_template_parts' src/monomorph/` → 0 | NOT MET — plan-105 not started |
| Feature worktree, baseline gate, green suite, perf baseline | as plan-104-A §Prerequisites (fresh capture for 106) | NOT MET — run first |

Both plan dependencies are hard gates: 106's letters assume `ParameterType`
below the IR (104) and one grammar + `UserOf` (105); without them the census
cannot reach zero and the typed engines would re-shim.

## 1. Goal

- `ir::lower::expression_type(…) -> Option<ParameterType>`; its environment
  maps (`function_returns`, `function_types`, `function_params.CallParam.type_`,
  `declared_binding_types`, `binding_types`, the per-body `locals`) hold
  `ParameterType`; the 27 `.name()` read-shims (plan-102 residue) and the two
  local numeric-promotion copies are gone.
- Monomorph's `expression_type` and `FunctionContext` are typed likewise; its
  two promotion copies are gone.
- Both engines call ONE typed promotion function in `src/numeric.rs`
  (signature over `ParameterType` scalars), replacing 4 of the measured 6
  copies (`rg -n 'fn (numeric_binary_result_type|promote_loop_numeric_type_name)' src/`
  → 6 today: monomorph×2, ir/lower×2, codegen type_utils×1, syntaxcheck×1;
  the codegen copy falls in plan-104, the syntaxcheck copy in 106-C/E).

### Non-goals (explicit constraints)

- No change to compiled output — byte-identical vs the 106 baseline (pure
  representation swap; the gate is the check, a diff is a bug to root-cause).
- No diagnostics wording/order change (messages render `name()` at format
  sites).
- Do not merge the two engines into one (they serve different phases —
  monomorph pre-instantiation, lower post-); E consolidates their SHARED
  algebra (promotion, literal classification), not their walks.
- `ir::verify` (B) and syntaxcheck (C/D) untouched here.

## 2. Current State

Post-plan-102, both engines read typed HIR nodes but immediately render
`.name()` into `String` environments and infer over strings — the deliberate,
recorded C3/D3 staging residue. The maps and returns are `String` end to end
(`rg -n 'Option<String>' src/ir/lower.rs src/monomorph/lower.rs | grep -c
expression_type`-adjacent signatures; `FunctionContext` at
`src/monomorph/mod.rs:88-103` is all `HashMap<String, String>`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `ir::lower` `.name()` render-shims | 27 | `rg -c '\.name\(\)' src/ir/lower.rs` → 27 |
| monomorph `.to_string()` type churn | 100–105 | `rg -c '\.to_string\(\)' src/monomorph/ \| awk -F: '{s+=$2} END{print s}'` (≈105 at plan-writing; re-measure at kickoff post-105) |
| duplicate promotion fns (this letter kills 4) | 6 | `rg -n 'fn (numeric_binary_result_type\|promote_loop_numeric_type_name)' src/` → 6 |
| `expression_type` engines retyped | 2 | `src/ir/lower.rs:1940`, `src/monomorph/lower.rs:1823` |
| `FunctionContext` String maps | 5 fields | read `src/monomorph/mod.rs:88-103` |

### Verified properties

- **Both engines' string outputs are consumed only by their own layer's
  stores** (post-104/105 there is no downstream string consumer left except
  verify/syntaxcheck, which B/C retype). UNVERIFIED until the 104/105 gates are
  MET — re-verify at kickoff with a caller sweep of both `expression_type`s and
  record here.

## 3. Design Overview

The same inside-out conversion proven in plan-102-E and plan-104-B: stores
first (maps → `ParameterType`), then the engine signatures, then the read
sites — one letter, one gate. The typed promotion function lands in
`src/numeric.rs` beside the base algebra so E's consolidation has its single
source already in place.

**Correctness risk:** the promotion unification — four hand-copies collapsing
to one must reproduce every pair's result exactly (the review warns drift here
is "a silent wrong value"). Mitigation: an exhaustive pairwise unit test over
the scalar lattice (all operator × type-pair combinations) asserting the new
single source equals each old copy's table, written BEFORE deleting the copies.

### Rejected alternatives

- **Merge the two engines outright.** Rejected: they answer different
  questions at different pipeline phases; forcing one walk now would braid E's
  consolidation concern into a retype letter.

## Compatibility / Format Impact

None externally observable.

## Phases

### Phase 1 — one typed promotion source (+ equivalence proof)

- [ ] Add the typed promotion fn(s) to `src/numeric.rs`; unit-test exhaustive
      equivalence against the ir/lower + monomorph copies (every operator ×
      scalar pair).
- [ ] Tests green with the copies still in place (no behavior change yet).

Acceptance: equivalence tests pass; suite green.
Commit: —

### Phase 2 — monomorph engine typed

- [ ] `FunctionContext` maps + `enclosing_return` → `ParameterType`;
      `expression_type` → `Option<ParameterType>`; read sites native; the two
      local promotion copies deleted (call numeric.rs).
- [ ] Tests: monomorph unit suite; generics fixtures (incl. plan-105-B's).

Acceptance: suite green; `artifact-gate all` no NEW diff.
Commit: —

### Phase 3 — ir::lower engine typed

- [ ] The five String maps + per-body `locals` → `ParameterType`;
      `expression_type`/`match_expression_type`/`literal_expression_type` →
      typed; the 27 `.name()` shims removed or annotated (diagnostic formatting
      only); the two local promotion copies deleted.
- [ ] Tests: ir unit suite; full corpus.

Acceptance: suite green; `artifact-gate all` no NEW diff; `rg -c '\.name\(\)'
src/ir/lower.rs` → each survivor is a render-out (diagnostic/serializer) site,
listed here; **no-backward check**: no `ParameterType::parse` of a value that
was rendered from a `ParameterType` (grep per plan-104-A's acceptance pattern).
Commit: —

## Validation Plan

- Tests: promotion-equivalence units; both engines' unit suites; full corpus.
- Coverage check: every fixture flows both engines (monomorph pre, lower post).
- Runtime proof: `artifact-gate all` byte-identical; `test-accept` no NEW
  mismatch (2 documented environmental failures excepted).
- Doc sync: none in A (E owns the docs pass).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Typed promotion signature:** over `ParameterType` directly vs a small
  `NumericClass` enum extracted first. Recommend `ParameterType` directly
  (scalars are variants; no intermediate abstraction).

## Corrections

<Filled in during execution.>

## Summary

The two mid-pipeline engines stop speaking strings, and the promotion algebra
gets its single source with an exhaustive equivalence proof — the review's
"silent wrong value" class addressed by construction, all behind the 0-diff
gate. B–E finish the checker, delete the backward render, and certify the
terminal invariant.
