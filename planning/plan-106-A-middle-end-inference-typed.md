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
| **E** | Consolidation (one numeric-promotion source, codegen sibling walks merged) + the terminal no-strings census | ~~medium–large~~ **large** (Correction 2) |

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
| plan-104 complete (NIR + codegen typed) | plan-104-A..D archived; `rg -c 'type_: String' src/target/shared/nir/mod.rs` → 0 | **MET** 2026-08-24 — `ls planning/completed/ \| grep plan-104` → A/B/C/D all archived; the `rg -c` exits 1 with no output (0 matches) |
| plan-105 complete (driver + one grammar) | plan-105-A/B archived; `rg -n 'rsplit_once\(" AS "\)' src/` → 0; `rg -n 'user_template_parts' src/monomorph/` → 0 | **MET** 2026-08-24 — both archived; both greps return only *historical comments / a test name* (`manifest/package.rs:1205`, `ir/types.rs:92`; `monomorph/helpers.rs:286,824,899,901`), 0 code sites |
| Feature worktree, baseline gate, green suite, perf baseline | as plan-104-A §Prerequisites (fresh capture for 106) | **MET** 2026-08-24 — worktree `.claude/worktrees/P-106` on `worktree-P-106` (base `94a38078b`); `scripts/artifact-gate.sh target/release/mfb all` → `1255 tests, 1402 build(s), 1730 golden(s) checked, 0 diff(s)` (recorded in `planning/plan-106-baseline-diffs.txt`); `rustup run 1.96.0 cargo test --no-fail-fast` → exit 0, 0 FAILED; `scripts/bench-lowering.sh` recorded in `planning/plan-106-bench-baseline.txt` |

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

Re-measured at kickoff (2026-08-24, base `94a38078b`) and again at close.

| What | At plan-writing | At kickoff | After A | Command |
|---|---|---|---|---|
| `ir::lower` `.name()` render-shims | 27 | 31 | **16**, all render-out (each listed in Phase 3) | `rg -c '\.name\(\)' src/ir/lower.rs` |
| monomorph `.to_string()` type churn | 100–105 | 105 | 79 | `rg -c '\.to_string\(\)' src/monomorph/ \| awk -F: '{s+=$2} END{print s}'` |
| duplicate promotion fns (this letter kills 4) | 6 | **7** (+1 typed twin = 8 — Correction 1) | **4** (syntaxcheck ×2 → C; codegen ×2 → E) | `rg -n 'fn (numeric_binary_result_type\|promote_loop_numeric_type)' src/` |
| `expression_type` engines retyped | 2 | 2 | 2 (both `-> Option<ParameterType>`) | `src/ir/lower.rs`, `src/monomorph/lower.rs` |
| `FunctionContext` String maps | 5 fields | 5 fields | 0 | read `src/monomorph/mod.rs` |
| `LowerContext` String type stores | — | 4 fields + 17 `locals` signatures | 0 | read `src/ir/lower.rs` |
| hand-rolled type-grammar copies in `ir::lower` | — | 4 | **0** (all deleted in Phase 3) | `parse_map_type`, `parse_map_entry_type`, `function_type_parts_for_predicate`, `collection_iteration_type` |

### Verified properties

- **Both engines' string outputs are consumed only by their own layer's
  stores** (post-104/105 there is no downstream string consumer left except
  verify/syntaxcheck, which B/C retype). **VERIFIED** by construction during the
  conversion: both `expression_type`s were retyped to `Option<ParameterType>`
  and the compiler enumerated every consumer (16 callers in `ir::lower`, 16 in
  `monomorph`). None reached outside its own layer — no change was required in
  `ir::verify`, `syntaxcheck`, `resolver`, or `hir`, and `git diff --stat`
  confirms those trees are untouched. The only cross-layer edits were *adding*
  typed accessors in `codegen::{registry,builtins}` (the engines' downstream
  queries) and the one descriptor bug-fix in Correction 6.

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

- [x] Add the typed promotion fn(s) to `src/numeric.rs`; unit-test exhaustive
      equivalence against the ir/lower + monomorph copies (every operator ×
      scalar pair).
      **Landed:** `typed_binary_result_type`, `typed_money_result_type`,
      `typed_promote_loop_numeric_type`, `is_numeric` — the table itself, over
      `ParameterType` variants. The name-keyed `binary_result_type` /
      `money_result_type` / the new `promote_loop_numeric_type_name` are now
      *adapters* over them (`numeric_variant` / `numeric_variant_name`, a closed
      five-arm match — **not** `ParameterType::parse`), so exactly one table
      exists. Equivalence is pinned against **frozen verbatim copies** of the
      pre-change bodies (`legacy_*` in the test module) rather than against the
      shipped adapters, which now delegate and would make the comparison
      vacuous: 17 operators × 10 × 10 operand pairs = **1,700** binary
      assertions, 10³ = **1,000** loop-fold assertions, plus the full Money
      dimensional table and the `is_numeric` lattice.
- [x] Tests green with the copies still in place (no behavior change yet).
      `rustup run 1.96.0 cargo test --bin mfb numeric::` → 28 passed, 0 failed;
      `… monomorph::helpers` → 22 passed, 0 failed.

Two extra tasks this phase needed (both required to leave no dead code, per
AGENTS.md "No blanket dead-code suppression"):

- [x] Delete the two hand-copied `promote_loop_numeric_type_name` bodies
      (`ir/lower.rs:1583`, `monomorph/helpers.rs:551`) and repoint their call
      sites (`ir/lower.rs:898`, `monomorph/lower.rs:1134`, and
      `monomorph/helpers.rs`'s two test assertions) at `numeric::`. Promotion
      definitions: **7 → 5** (see Correction 1).
- [x] Delete the name-keyed `numeric::is_numeric_type`, which lost its only
      production caller when `binary_result_type` became an adapter over
      `numeric_variant`. Its two tests are retargeted onto `is_numeric` /
      `numeric_variant` and a frozen `legacy_is_numeric_type` oracle.
      `cargo build --bin mfb` → **0 warnings**.

Acceptance: equivalence tests pass; suite green. **MET** — the equivalence
suite above, plus the Phase-2 full-suite/gate/test-accept runs on the same tree.
Commit: `f20b96ca9` (shared with Phase 2 — see Correction 5)

### Phase 2 — monomorph engine typed

- [x] `FunctionContext` maps + `enclosing_return` → `ParameterType`;
      `expression_type` → `Option<ParameterType>`; read sites native; the two
      local promotion copies deleted (call numeric.rs).
- [x] Tests: monomorph unit suite; generics fixtures (incl. plan-105-B's).
      `cargo test --bin mfb monomorph` → 61 passed, 0 failed.

What the retype actually reached (the read sites are not confined to
`expression_type` — each of these had to convert or the engine would not
compile without a new render shim):

- [x] `FunctionContext`: `locals`/`function_returns`/`function_types`/`globals`
      values and `enclosing_return` → `ParameterType`. Keys stay `String` (they
      are NAMES) per the plan's Open Decision in letter B.
- [x] `expression_type` → `Option<ParameterType>`, every arm structural: the
      `format!("List OF {…}")` / `"Set OF …"` / `"Map OF … TO …"` /
      `"FUNC(…) AS …"` constructions became `ListOf`/`SetOf`/`MapOf`/`Func`
      variants, and the scalar arms became variants instead of
      `"Integer".to_string()`.
- [x] `helpers::opt_type_name` → `opt_type` (returns the type); the render form
      survives as a two-line wrapper for the **symbol-mangling** sites
      (`mangle_name`, `overload_key`), which are render-out by definition.
- [x] `params_match` compares `ParameterType`s structurally.
- [x] The `expected_type` thread — `lower_expression`, `lower_constructor_arg`,
      `resolve_overload`, `arg_slot_expected`, `constructor_arg_field_type`,
      `expected_element_type` — is `Option<&ParameterType>`. This **deleted a
      backward seam**: the constructor arm did
      `expected_type.map(ParameterType::parse)` on a value that had been
      rendered from a `ParameterType` two frames up (`lower.rs:1362`).
- [x] The `arg_types` chain — `arg_types_in_param_order`,
      `instantiate_function`, `resolve_general_builtin_override`,
      `resolve_imported_overload` — takes `&[ParameterType]`. The generic-
      inference `unify_type` call in the constructor arm no longer re-parses
      (`unify_type(&field.type_, &actual, …)` directly).
- [x] Builtin return-type resolution goes through the TYPED registry entry
      (`resolve_call_return_type_typed`, plan-104-C) in both
      `builtin_call_return_type` and `resolve_general_builtin_override`, so
      neither the argument types nor the resolved return crosses a string.
- [x] `ForEach`'s element type matches the iterable's `ParameterType` directly
      instead of re-parsing its rendered name.
- [x] The two byte-identical copies of the function-signature construction in
      `function_context` / `add_function_to_context` collapse into one
      `helpers::function_signature_types`.
- [x] Two renders deliberately KEPT, each documented at its site:
      `resolve_imported_overload` compares against a **decoded package
      signature** (`ImportedOverload::param_types`, wire strings) using
      `normalize_type`/`types_compatible` — qualifier stripping and positional
      `Unknown` wildcarding, neither expressible structurally, so the call's
      argument renders at that wire boundary; and `instantiate_function`
      renders for `template_view_type`, whose result also spells the
      `TYPE_CALL_ARGUMENT_MISMATCH` message (letter E retypes it).

Acceptance: suite green; `artifact-gate all` no NEW diff. **MET**:
`cargo test --bin mfb` → 3644 passed, 0 failed;
`scripts/artifact-gate.sh target/release/mfb all` →
`1255 tests, 1402 build(s), 1730 golden(s) checked, 0 diff(s)` — byte-identical
to the 106 baseline. Plus `cargo test --no-fail-fast` exit 0 (62 suites, 0
FAILED) and `test-accept` 1271 ran / 0 mismatches.
Commit: `f20b96ca9` (shared with Phase 1 — see Correction 5)

### Phase 3 — ir::lower engine typed

- [x] The five String maps + per-body `locals` → `ParameterType`;
      `expression_type`/`match_expression_type`/`literal_expression_type` →
      typed; the `.name()` shims removed or annotated; the two local promotion
      copies deleted.
- [x] Tests: ir unit suite; full corpus.

What landed:

- [x] `LowerContext`'s `function_returns` / `function_types` /
      `function_params.CallParam.type_` / `binding_types` /
      `current_return_type`, the four builders that fill them
      (`function_returns`, `function_types`, `function_params`,
      `declared_binding_types`), and all **17** `locals: &HashMap<String, …>`
      signatures → `ParameterType`. `RecoverTarget`, `CapturedLocal` and
      `InlineTrapTarget::Bind` retyped with them.
- [x] `expression_type` → `Option<ParameterType>`, structural in every arm;
      `match_expression_type` and `literal_expression_type` likewise;
      `TypeIndex::constructor_result` / `record_field_type` typed.
- [x] The whole `expected` thread (`lower_expression_with_expected`,
      `wrap_union_value`, `call_argument_expected_type`,
      `lower_constructor_args`) is `Option<&ParameterType>`; the numeric-literal
      coercion compares `Some(&ParameterType::Fixed)` instead of `Some("Fixed")`.
- [x] **Four hand-rolled type-grammar copies deleted from `ir::lower`**:
      `parse_map_type`, `parse_map_entry_type`,
      `function_type_parts_for_predicate` (a `strip_prefix("FUNC(")` +
      `split_once(") AS ")` that cut at the FIRST `") AS "`, mis-typing a
      higher-order parameter), and `collection_iteration_type`'s
      `List OF`/`Set OF`/`Map OF` + `RES ` cascade — now a variant match plus a
      `strip_res` helper. `numeric_binary_result_type` and
      `declared_func_parts` are gone too (the engine calls
      `numeric::typed_binary_result_type` and matches `Func` directly).
- [x] `numeric::promote_loop_numeric_type_name` — the name-keyed adapter Phase 1
      added — **deleted**: with both engines typed it had no callers left.
      Promotion definitions: **7 → 4** (`syntaxcheck`×2 → letter C,
      `codegen/type_utils`×2 → letter E), all four delegating to the one
      `numeric.rs` algebra.
- [x] New typed accessors so the engine never renders to ask a question:
      `registry::constant_type` / `call_return_type_typed` /
      `argument_types_typed` / `default_argument_padding` (now
      `Vec<(ParameterType, _)>` — the descriptor already held the type),
      `builtins::package_constant_type` / `call_return_type` /
      `argument_types_typed`, `general::filter_predicate_type_typed`.
- [x] `ParameterType::with_state` added in `src/types.rs`: the structural
      equivalent of parsing `"{base} STATE {state}"`, replacing five
      `format!("{…} STATE {…}")` sites (function return, `RES` param, `LET`
      binding, trap binding, union match-case binding). Guarded by
      `with_state_matches_parse_of_the_concatenated_spelling` — 21 base shapes ×
      3 state types = 63 assertions that `t.with_state(s)` equals
      `parse(&format!("{} STATE {}", t.name(), s.name()))` **and** still renders
      to that spelling.
- [x] Fixed a latent descriptor bug the retype exposed, with a permanent guard
      (Correction 6).

`.name()` census — `rg -c '\.name\(\)' src/ir/lower.rs` → **16**, every one a
render-out into a NAME domain, listed:

| Site | Why it renders |
|---|---|
| `:1655`, `:1667`, `:1718`, `:1998`, `:3219` | union-variant / constructor name as a *symbol* (an `IrValue::Local` name, an index key) |
| `:1752`, `:2037` | reading the `STATE` clause, which rides inside the resource's nominal *spelling* (`parse` has no `STATE` arm) |
| `:2060`, `:3247`, `:3540` | `TypeIndex` lookups — keyed by type NAME (a declaration table) |
| `:2901`, `:2923`, `:2949`, `:2979`, `:3039` | per-package dispatch tables keyed by type name (`general_override_target`, `TLS_LISTENER_TYPE`, audio / vector / term selectors) |
| `:2547` | registry record-constant field types, into the name-keyed constant path |

**No-backward check**: `rg -n 'ParameterType::parse' src/ir/lower.rs`
(production) → every remaining site is a *source/wire/descriptor* parse-in, not
a re-parse of a render: `native_type` + the two `return_state_type` sites (the
un-elaborated `HirItem::Link` AST block — see the fn doc), `lower_field`'s
`ImportedTypeField.type_` (wire-decoded package types), the registry
record-constant type name, and static literals. Zero parse-of-render.

Acceptance: suite green; `artifact-gate all` no NEW diff; the `.name()` census
recorded above; the no-backward check clean. **MET**:
`cargo test --bin mfb` → 3646 passed, 0 failed;
`scripts/artifact-gate.sh target/release/mfb all` →
`1255 tests, 1402 build(s), 1730 golden(s) checked, 0 diff(s)`.
Commit: —

## Validation Plan

- Tests: promotion-equivalence units; both engines' unit suites; full corpus.
- Coverage check: every fixture flows both engines (monomorph pre, lower post).
- Runtime proof: `artifact-gate all` byte-identical; `test-accept` fully green
  at **1271 ran, 0 mismatches** (the "2 documented environmental failures"
  clause is deleted — see Correction 4).
- Doc sync: none in A (E owns the docs pass).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Typed promotion signature:** over `ParameterType` directly vs a small
  `NumericClass` enum extracted first. Recommend `ParameterType` directly
  (scalars are variants; no intermediate abstraction).
  **RESOLVED as recommended** (Phase 1): `numeric::typed_binary_result_type`
  takes and returns `&ParameterType`/`ParameterType`. No intermediate enum was
  needed — the five numeric scalars are already variants, and the two private
  helpers the name-keyed adapters need (`numeric_variant` /
  `numeric_variant_name`) are five-arm matches over this module's own `TYPE_*`
  constants, not a new abstraction.

## Corrections

### 1. The promotion-copy population is 7, not 6 — the plan's own grep undercounts

The plan (§1 Goal, §2 Measured populations, and 106-E §1) measures the
duplication with

```
rg -n 'fn (numeric_binary_result_type|promote_loop_numeric_type_name)' src/'   → 6
```

That alternation misses `syntaxcheck::helpers::promote_loop_numeric_type`
(`src/syntaxcheck/helpers.rs:244`) — the loop fold, but named without the
`_name` suffix because syntaxcheck's copy returns its private `Type`, not a
string. The correct census command is the suffix-free pattern:

```
$ rg -n 'fn (numeric_binary_result_type|promote_loop_numeric_type)' src/
src/syntaxcheck/helpers.rs:244:pub(super) fn promote_loop_numeric_type(...) -> Type
src/syntaxcheck/helpers.rs:272:pub(super) fn numeric_binary_result_type(..., &Type, &Type) -> Type
src/monomorph/helpers.rs:547:  pub(super) fn numeric_binary_result_type(..., &str, &str) -> &'static str
src/monomorph/helpers.rs:551:  pub(super) fn promote_loop_numeric_type_name(...) -> String
src/ir/lower.rs:1583:          fn promote_loop_numeric_type_name(...) -> String
src/ir/lower.rs:3655:          fn numeric_binary_result_type(..., &str, &str) -> &'static str
src/codegen/engine/types/type_utils.rs:462: pub(crate) fn numeric_binary_result_type(...)
                                                                              → 7
```

Plus an eighth promotion-shaped function the pattern also misses,
`type_utils::typed_numeric_binary_result_type` (plan-104-B's typed twin, which
renders `name()` and re-parses the string result). Letter E's "→ exactly 1
definition" acceptance is re-scoped against **8**, using the suffix-free grep
above widened with `typed_`. Phase 1 took it to 5 (+ the typed twin) by
deleting the two `promote_loop_numeric_type_name` hand-copies.

Scope impact on other letters: none re-derived from the wrong number — A's goal
text says "replacing 4 of the measured 6", and A still deletes exactly those 4
(2 in Phase 1, 2 across Phases 2–3). C already owned syntaxcheck's copies; the
correction only means C owns **two** (`numeric_binary_result_type` *and*
`promote_loop_numeric_type`), and E's final count is 1 production definition,
not "1 of 6".

### 6. A REAL BUG the retype exposed: `fs::pathJoin`'s parameter descriptor

Phase 3's first gate run was **not** byte-identical — 1 DIFF, on
`rt-behavior/project/project-fs-createTempFile-package-valid`'s `.ir`. Per the
skill this is a root-cause trigger, not a stop; diffing that one fixture
localized it immediately:

```
$ diff golden/project_fs_createTempFile_package_valid.ir <rebuilt>
32c32
< … "target": "fs.pathJoin", "args": [{ "kind": "list", "type": "List OF String", …
> … "target": "fs.pathJoin", "args": [{ "kind": "list", "type": "List OF Unknown", …
```

Cause — a latent **descriptor bug**, not the retype:

```
$ rg -n 'ParameterType::named\("[^"]* [^"]*"\)' src/          # ONE hit, tree-wide
src/codegen/builtins/fs/func_path_join.rs:83: ty: ParameterType::named("List OF String")
```

`named` is for a bare nominal; `Named("List OF String")` is a *different value*
from `list_of(String)` with an identical rendering. Every pre-plan-106 consumer
of `registry::argument_types` rendered `.name()` and re-parsed, which silently
normalized it in transit. plan-106-A's typed accessor
(`argument_types_typed`) hands back the raw variant, so `ir::lower`'s
`Some(ParameterType::ListOf(element))` match missed and the element type of
`fs::pathJoin([a, b])` fell back to `Unknown`.

Fixed at the descriptor (`ty: ParameterType::list_of(ParameterType::String)`) —
the gate is 0 diffs again. This is the class of latent defect the whole
"NO STRINGS" invariant exists to surface: a render→parse round trip was
laundering a wrong value, and it stayed invisible for exactly as long as
everything spoke strings.

- [x] Added a permanent guard, `registry::tests::
      descriptor_named_types_are_bare_nominals`: it walks EVERY registered
      descriptor (params, `Fill` defaults, returns, record fields — recursing
      into containers) and asserts no `Named(n)` has a name that re-parses to
      something structured. Verified RED against the original descriptor
      (`fs.pathJoin impl 0 param 'parts': 'List OF String' is a bare Named but
      its own spelling parses to ListOf(String)`) and GREEN after the fix.
      Deliberately scoped to `Named` rather than "every type round-trips":
      `Var` and `Arg` render as bare names and re-parse as `Named` **by
      design** (`parse` classifies grammar and cannot know a name is a type
      variable without the declaring scope), so a blanket round-trip assertion
      reds on all 16 thread-package type variables. Those are sanctioned; a
      structure-spelling `Named` is not.

### 7. The descriptor fix restores a MISSING diagnostic (one golden regenerated)

Correction 6's descriptor fix has one visible consequence: `test-accept` went to
**1 mismatch / 1271 ran** on `syntax/fs/func_fs_pathJoin_invalid`. With the
parameter now a real `ListOf(String)` instead of an opaque
`Named("List OF String")`, the registry matcher recurses into the element and
`fs::pathJoin([1, 2])` picks up the `TYPE_CALL_ARGUMENT_MISMATCH` it had been
escaping.

A's non-goals say "no diagnostics wording/order change", so this needed the
AGENTS.md four-question gate before the golden could move. It clears it:

1. **When/why written.** `git log -- tests/syntax/fs/func_fs_pathJoin_invalid/`
   → `e76b2b741` "regenerate 16 stale sidecar goldens after os/fs migration".
   The golden was *regenerated from* the post-migration compiler, never
   hand-asserted — it captured whatever the migration produced.
2. **Behavior it protects.** That `fs::pathJoin` rejects each bad call shape:
   arity-0, a scalar argument, a wrong-element list, arity-2.
3. **Who else depends.** Only this fixture's own `build.log`
   (`artifact-gate all` is 0 diffs; the other 1270 acceptance tests are green).
4. **Proof it is wrong.** The sibling `fs::writeBytes`, whose list parameter is
   correctly declared `ty: ParameterType::list_of(ParameterType::Byte)`, DOES
   report the argument mismatch for the identical shape:

   ```
   $ grep TYPE_CALL_ARGUMENT_MISMATCH -A1 \
       tests/syntax/fs/func_fs_writeBytes_invalid/golden/build.log
   …:5 error[…TYPE_CALL_ARGUMENT_MISMATCH]: …
      Call to `fs.writeBytes` has argument type(s) (Integer, List OF Integer),
      expected String, List OF Byte.
   ```

   Every other `fs` list parameter uses `list_of` (`rg -n 'ty:
   ParameterType::list_of' src/codegen/builtins/fs/*.rs` → `write_all_bytes`,
   `write_bytes_atomic`, `write_bytes`, `append_bytes`); `pathJoin` was a
   one-off slip **in that same migration commit** (`73374e779`). So the golden
   recorded a compiler defect, and the two fixtures disagreed for no reason but
   the typo.

**The accept/reject set does not change.** `fs::pathJoin([1, 2])` was already
rejected — via `TYPE_LIST_ELEMENT_MISMATCH` ("List element has type Integer,
expected String"), which the golden kept and still keeps. The regenerated
golden is **purely additive**: `git diff tests/ | grep -c '^-[^-]'` → **0**
removed lines, 12 added. Nothing was re-baselined away; a missing diagnostic
was restored.

- [x] Regenerated exactly that one golden
      (`scripts/sync-goldens.sh target/release/mfb func_fs_pathJoin_invalid`
      → "synced 1 golden file(s) across 1 test(s)").

### 5. Phases 1 and 2 land in ONE commit (they cannot be split)

The plan gives Phases 1 and 2 separate `Commit:` lines. They share a hash
because a Phase-1-only tree does not compile:

Phase 1's dead-code obligation (AGENTS.md forbids shipping an unused function
or a blanket `#[allow(dead_code)]`) required *deleting* the two hand-copied
`promote_loop_numeric_type_name` bodies and repointing their call sites — which
lives in `src/monomorph/helpers.rs` and `src/monomorph/lower.rs`, the same two
files Phase 2 then retypes. Committing only `src/numeric.rs` +
`src/ir/lower.rs` would leave `monomorph` calling a function that no longer
exists in its module.

Both phases are independently *verified* (Phase 1: the 2,700-assertion
equivalence suite green with every engine still stringly; Phase 2: full suite +
byte-identical gate); only the commit boundary is shared.

### 4. `test-accept` has NO environmental failures — the "2 documented" ones are fixed

Every letter's Validation Plan says "`test-accept` no NEW mismatch (2 documented
environmental failures excepted)", inherited from plan-104-A's "Known
pre-existing noise" note (the 5 stdin-EOF `acceptance` io sub-tests and the
`project_name` deep-worktree path bug). Both were a **real harness bug** — a
bare `mfb test` consuming the find-pipe's stdin, which also silently skipped 72
fixtures — fixed 2026-08-24 before this plan started. Measured at 106's base:

```
$ scripts/test-accept.sh target/release/mfb /tmp/p106-accept-out
…
acceptance tests passed (1271 test(s) ran)
exit=0
```

The exception clause is therefore **deleted** for plan-106: every letter's
acceptance is `test-accept` fully green at **1271 ran, 0 mismatches**, and the
`N ran` count is watched between runs (a drop means fixtures were skipped, not
that they passed).

### 3. Monomorph's substitution walk is a parse↔render machine — assigned to E

A's Goal names `expression_type` and `FunctionContext`. Retyping those does not
by itself satisfy the Phase-3 "no-backward check", because monomorph's
*substitution* walk sits beside them and is string-in/string-out:

```
$ rg -n 'fn concrete_type_name|fn template_view_type' src/monomorph/lower.rs
src/monomorph/lower.rs:1630:  fn concrete_type_name(&mut self, type_name: &str, subs) -> String
src/monomorph/lower.rs:1738:  fn template_view_type(&self, type_name: &str) -> String
```

Both `ParameterType::parse` their input, recurse **by rendered child name**, and
`format!("List OF {…}")` the result back — 14 `format!` type-constructions
between them. Their callers then re-parse (`ParameterType::parse(&self
.concrete_type_name(…))` at `lower.rs:421,475,481,862,1069`). This is
pre-existing (plan-105-B converted the *classification* to the canonical
grammar but left the string carrier), NOT introduced by A.

Assignment: **letter E**, whose stated job is the census and straggler
burn-down, and which already owns the `format!("List OF …")` census line. It is
listed as an explicit task in E §Phases Phase 2 (not "future work"). A's Phase-3
no-backward check is therefore recorded as "0 NEW parse-of-render; the
surviving sites are exactly `concrete_type_name`'s five callers, enumerated
above and closed by E".

Rationale for not braiding it into A Phase 2: the recursion is deliberately
*by name* (the `strip_type_group` unwrap per level and the `substitutions`
lookup keyed on the whole child spelling both depend on it — see the comment at
`lower.rs:1659-1664`), so retyping it is a behavioral change to freshly-landed
plan-105-B code, not a mechanical carrier swap. Converting it in the same diff
as the `FunctionContext` retype would make a golden failure unattributable —
exactly the reason C and D are separate letters.

### 2. Plan-104 did NOT eliminate codegen's string type-grammar parsing

A §1 and E §1 both assume "the codegen copy falls in plan-104". Measured at
this plan's base (`94a38078b`, plan-104-A..D all archived):

```
$ rg -n 'fn numeric_binary_result_type' src/codegen/
src/codegen/engine/types/type_utils.rs:462   # still &str -> &'static str
```

and the wider hand-rolled-grammar census still hits codegen in ~15 places
(`rg -n 'strip_prefix\("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF )' src/`
→ `codegen/engine/types/type_utils.rs` ×5, `codegen/memory/arena/…` ×2,
`codegen/collection/layout/…` ×2, `codegen/cleanup/owned/…` ×2,
`codegen/memory/{data,value}/…` ×2, `codegen/engine/{value,validation}/…` ×2,
`codegen/builtins/math/gen_math.rs` ×1). Plan-104 typed the NIR *data model*
and the engine's hot paths; it did not reach zero on the grammar census.

This is **not** a prerequisite failure — 104's own acceptance never claimed the
census — but it is scope E inherits. E's Phase 2 "straggler burn-down" is
therefore *not* the small mop-up the plan estimated (medium, 1h–2h); it carries
codegen's residual grammar sites. E's Effort line is corrected to **large**
below, and the stragglers are enumerated as explicit tasks in E when Phase 2
runs its census. They are tasks, not deferrals.

## Summary

The two mid-pipeline engines stop speaking strings, and the promotion algebra
gets its single source with an exhaustive equivalence proof — the review's
"silent wrong value" class addressed by construction, all behind the 0-diff
gate. B–E finish the checker, delete the backward render, and certify the
terminal invariant.
