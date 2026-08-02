# plan-72-AA: vector package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/vector.rs` (770 LOC, 7 metadata helpers, 1
`package_source_glue!`, 1 builtin-type helper, 1 custom-resolver helper,
23 fixtures) to a `pub(crate) static VECTOR: BuiltinModule` with a
`VectorResolver` that preserves typed `implementation_name`.

References: plan-72 overview, `src/builtins/vector.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/vector/`,
`memory/plan-32-execution-state.md` (RVV context — do not regress).

## Goal

- `vector::VECTOR` descriptor exists with `VectorResolver` covering
  `implementation_name(name, arg_types)` (line 248).
- Legacy free functions in `vector.rs` become wrappers over
  `VECTOR`/`VectorResolver`.
- Parity tests cover every function, the builtin type, and every
  arg-type driven implementation-name case.

## Non-goals

- Do not change RVV or dual-path codegen semantics
  (`[[plan-32-execution-state]]`).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/vector.rs` has 7 helpers, one builtin type (line 41), a
`package_source_glue!` companion, and a typed `implementation_name` at
line 248. Fixture load is 23 projects.

## Phases

### Phase AA1 — descriptor and resolver

- [x] Add `pub(crate) static VECTOR: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default. (19 function members, single
      overloads, `ReturnType::Custom`; `cross` widens 1..3 via two trailing
      `Optional` operands. The 42 dynamic constants stay hand-authored — see
      Corrections.)
- [x] Add `BuiltinType` entry for the vector builtin type. (All nine value
      records `Float2/3/4`, `Fixed2/3/4`, `Integer2/3/4`, `Record` with empty
      fields like `net::Url`.)
- [x] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [x] Implement `VectorResolver` for typed `implementation_name` (and
      `resolve_return_type`), delegating to the hand-authored helpers.
- [x] Rewrite the 7 metadata helpers as wrappers over
      `VECTOR`/`VectorResolver`. (`is_vector_function` → `contains`, `arity` →
      `DefaultResolver` with the constant branch kept; `resolve_call`/
      `implementation_name`/`expected_arguments`/`call_param_names`/
      `is_builtin_type`/constant surface stay hand-authored — see Corrections.)
- [x] Register `VECTOR` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests: every `vector.*` function name, the builtin type, and
      every typed implementation-name case (element × dimension, scalar/vector
      returns, `cross`-by-dimension, and a package constant) via resolver samples.

Acceptance: `cargo test` passes; every `vector.*` fixture runs clean
under `scripts/test-accept.sh target/debug/mfb target/accept-actual`,
including the existing `tests/byte-identity/vector` cohort.
(`cargo test --bin mfb builtins::` → 417 passed; full acceptance +
byte-identity at finalization.)
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `vector` fixtures per the overview.

## Corrections

- **The 42 package constants stay hand-authored; the descriptor models only the
  19 function members.** `vector.zeroFloat3`, `vector.upInteger2`, … are parsed
  dynamically from `<base><Element><Dim>` (`is_vector_constant`/`parse_constant`),
  not enumerated, so they cannot be static descriptor functions. `is_vector_call`
  stays `is_vector_function || is_vector_constant`, and `arity` keeps its
  `is_vector_constant → (0,0)` branch before delegating function arities to
  `DefaultResolver::arity`. The constant flow is still exercised through the
  resolver (a `vector.zeroInteger2` resolver sample), since the resolver's
  `resolve_return_type`/`implementation_name` delegate to the constant-aware
  `resolve_call`/`implementation_name`.
- **`VectorResolver` wraps the kept `resolve_call`/`implementation_name` rather
  than replacing them.** vector's returns and monomorph targets are
  argument-dependent (`ReturnType::Custom`, `Implementation::Custom`), so a
  resolver is required for the parity harness to skip the data-only return/impl
  checks and drive `resolver_samples` instead. The resolver delegates to the
  existing hand-authored helpers (the `net`-style minimal-churn choice), so
  every typed overload resolves byte-identically; BB can then route `vector::`
  return types and monomorph targets through the registry.
- **Repointed the two "unmigrated-example" tests — they had no target left.**
  `descriptor::production_registry_holds_migrated_packages` and
  `mod::adapters_fall_back_for_unmigrated_packages` both used `tls` as a
  still-unmigrated real package (per the plan-72 migration-mechanics note that Z
  must repoint them). With thread/tls/vector now registered, ALL 26 packages are
  migrated and no unmigrated real example exists. `production_registry_holds…`
  now asserts the registry is complete (26 modules, thread/tls/vector present);
  `adapters_fall_back…` (renamed `adapters_fall_back_on_registry_miss`) proves
  the still-live fallback mechanism with a synthetic `nonesuch.*` name the
  registry misses. Evidence: `cargo test --bin mfb builtins::` → 417 passed.
