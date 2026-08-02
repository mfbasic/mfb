# plan-72-E: collections package descriptor

Last updated: 2026-08-01 (premise corrected during execution — see Corrections)
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/collections.rs` (1355 LOC, 10 metadata helpers, 0
`package_source_glue!` macro uses but a hand-written source companion, 0 builtin
types, 50 fixtures) to a `pub(crate) static COLLECTIONS: BuiltinModule`.

**Corrected premise:** collections is NOT data-shaped. Its `resolve_call` is
deeply generic and type-dependent (`get(List OF T, Integer) → T`,
`keys(Map OF K TO V) → List OF K`, function-type matching for
`forEach`/`transform`/`filter`/`reduce`), dispatching to ~25 `resolve_*`
helpers. It is the most custom-resolution package in the plan; the census
`custom` column (which counts only `implementation_name`/overload helpers) missed
it. It ALSO injects a `.mfb` source companion (hand-written `source_file` /
`augmented_project`, opting out of the `package_source_glue!` macro because
`augmented_project` takes `AstProject` by value). So this letter is a
**resolver-backed** migration with a source — not a data-only one. The descriptor
model supports exactly this via `BuiltinResolver::resolve_return_type` (the same
hook the plan reserves for `H` datetime / `I` encoding).

References: plan-72 overview, `src/builtins/collections.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/collections/`,
`memory/collection-memory-mgmt.md`.

## Goal

- `collections::COLLECTIONS` descriptor exists and mirrors every current
  metadata helper.
- Legacy free functions become wrappers over `COLLECTIONS`.
- Parity tests cover every function, including MUT-append and shrink-related
  overloads whose behavior is load-bearing per the collection memory notes.

## Non-goals

- Do not change collection memory semantics (`[[collection-memory-mgmt]]`).
- Do not remove the wrapper functions; `BB` owns deletion.
- Do not merge with `general`.

## Current State

`src/builtins/collections.rs` is the largest data-shaped builtin at 1355 LOC
with 10 descriptor-owned helpers and no source companion or custom resolver.
Fixture load is 50 projects — the largest cohort in this plan after `fs`.

## Phases

### Phase E1 — descriptor and wrappers

- [x] Add `pub(crate) static COLLECTIONS: BuiltinModule` — 23 native-member
      functions with per-position parameter names + aliases (matching
      `call_param_names`) and arity (via `find`'s optional `start`); `return_type`
      `Custom` (generic, resolver-owned); a `WhenImported` `BuiltinSource`
      companion; and a `CollectionsResolver` whose `resolve_return_type` delegates
      to the retained `dispatch_resolve`. Parameter *types* are documentation only
      (a member's List/Map/Set overloads can't be one `ParameterType`). See
      Corrections re: the corrected (resolver-backed, sourced) premise.
- [x] Rewrite the metadata helpers over `COLLECTIONS`: `is_native_member_call`
      → `DefaultResolver::contains`; `arity` → `DefaultResolver::arity`;
      `resolve_call` routes through the descriptor resolver. `call_param_names`
      stays static (borrowed `&'static` ABI), PINNED to `COLLECTIONS` by the
      parity test. `expected_arguments` (custom "or"-phrased strings) and
      `call_return_type_name` (delegates to `general`) are not descriptor-derivable
      and stay as-is — documented.
- [x] Register `COLLECTIONS` with the `BuiltinRegistry`
      (`new(&[&app::APP, &bits::BITS, &collections::COLLECTIONS])`).
- [x] Parity tests: `parity_matches_descriptor` covers every native member +
      `collections.sort` (source generic, non-member) + `collections.nope`
      (membership, arity, param names/aliases), plus 12 resolver samples spanning
      List/Map/Set/generic return resolution, and the source rule.

Acceptance: `cargo test` passes; every `collections.*` fixture runs clean under
`scripts/test-accept.sh` (61 fixtures pass; byte-identity via the combined D+E
artifact-gate at finalization).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `collections` fixtures per the overview.

## Corrections

- **Premise "not because of custom behavior / no custom resolver" is FALSE.**
  `collections::resolve_call` is deeply generic and type-dependent (25 `resolve_*`
  helpers; `get(List OF T, Integer) → T`, `keys(Map OF K TO V) → List OF K`,
  function-type matching). The census `custom` column missed it because it counts
  only `implementation_name`/overload helpers, not `resolve_call` complexity.
  Corrected to a **resolver-backed** migration: return resolution lives on
  `CollectionsResolver::resolve_return_type` (delegating to the retained
  `dispatch_resolve`), exactly the mechanism reserved for `H`/`I`. The plan's
  design did not rest on collections being data-only — the descriptor model
  supports resolvers — so this is "claim false, correct and continue," not a
  core-premise stop.
- **Premise "no source companion / 0 source glue" is misleading.** collections
  injects a `.mfb` source companion via hand-written `source_file` /
  `augmented_project` (it opts out of the `package_source_glue!` macro because
  `augmented_project` takes `AstProject` by value, so `srcglue=0` for the macro is
  technically true). Modelled as `BuiltinSource { WhenImported, source_file }`.
- **Three facets are not descriptor-derivable for collections.**
  `call_param_names` stays static (borrowed `&'static` ABI), pinned by parity.
  `expected_arguments` keeps its hand-authored "or"-phrased strings (`"List OF T,
  Integer or Map OF K TO V, K"` — a single `ParameterType` can't express an
  overload alternation). `call_return_type_name` delegates to `general` (the
  override conventional type, not collections' own). All documented in code.
- **`DefaultResolver::param_names`/`arity` derive correctly** once `find`'s `start`
  is modelled optional (`DefaultValue::Fill`, inert — collections has no default
  padding). Parameter *types* in the descriptor are documentation only (List/Map/
  Set overloads share one name table).
- **Vocabulary refinement:** `parity::LegacySet::expected_arguments` made
  `Option` (like `argument_types`), so a package with a bespoke phrasing skips
  that assertion. Existing call sites (app, bits, A's two parity tests) wrapped in
  `Some(...)`.
- **`FUNCTIONS` (24 source generics: `sort`, set algebra, …) are NOT descriptor
  functions.** They are resolved by the monomorphizer, not `resolve_call`, so
  `is_collections_call`/`is_collections_function`/`is_native_member`/
  `native_member_bare`/`unary_callback_member` stay as-is (they span the source
  generics, which the descriptor does not model).
