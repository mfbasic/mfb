# plan-72-P: math package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/math.rs` (616 LOC, 6 metadata helpers, 0 source glue,
0 builtin types, 0 custom-resolver helpers, 45 fixtures) to a
`pub(crate) static MATH: BuiltinModule`.

References: plan-72 overview, `src/builtins/math.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/math/`,
`memory/clippy-fix-trims-double-double-constants.md`.

## Goal

- `math::MATH` descriptor exists and mirrors every current metadata helper.
- Legacy free functions in `math.rs` become wrappers over `MATH`.
- Parity tests cover every function name, arity, return type, and argument
  type.

## Non-goals

- Do not add builtin types or a source companion (math has neither today).
- Do not touch double-double constants (`[[clippy-fix-trims-double-double-constants]]`
  — do not run `clippy --fix` on `math.rs`).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/math.rs` is data-shaped with 6 descriptor-owned helpers, no
source companion, no builtin type, and no custom resolver. Fixture load is
45 projects.

## Phases

### Phase P1 — descriptor and wrappers

- [x] Add `pub(crate) static MATH: BuiltinModule` with every function (the 21
      callables; constants stay in `is_math_constant`), overload, parameter
      (canonical + aliases), return type (`Fixed(Integer)` for floor/ceil/round,
      `Fixed(Nothing)` for seed, `Custom` for every argument-type-preserving
      call), implementation (`Same`), and default (none).
- [x] Rewrite the 6 metadata helpers as wrappers over `MATH`
      (`is_math_call`/`arity`/`call_return_type_name` delegate;
      `call_param_names` borrowed static pinned by parity; `resolve_call` and
      `expected_arguments` stay hand-authored — see Corrections).
- [x] Register `MATH` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests for every `math.*` name and unknown-name behavior (plus a
      constant name, which the descriptor rejects).

Acceptance: `cargo test` passes; every `math.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: f47850c75

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `math` fixtures per the overview.

## Corrections

- **`resolve_call` and `expected_arguments` stay hand-authored.** math's
  `resolve_call` is argument-type-preserving (`abs(Integer) → Integer`,
  `abs(List OF Float) → List OF Float`, the SIMD array kernels) and its
  `expected_arguments` uses bespoke `"|"`-phrased type sets
  (`"Integer | Float | Fixed | Money"`). Neither is derivable from a single
  `ParameterType::Named`, so they remain hand-authored (parameter types in the
  descriptor are documentation only), like collections' resolver-owned
  resolution and io's `"no arguments"` phrasing. Only membership, arity, param
  names, and the nominal return type are descriptor-derived. math needs no
  resolver (it has no argument-dependent *implementation* selection — all
  `Implementation::Same`).
- **Repointed the `math`-as-unmigrated-example tests.** plan-72-A wrote two
  tests using `math` as the canonical *un*migrated package
  (`mod.rs:adapters_fall_back_for_unmigrated_packages` and
  `descriptor.rs:production_registry_holds_migrated_packages`). Migrating math
  makes `REGISTRY.module("math")` non-empty and routes `math.abs`'s type-set
  `expected_arguments` through the descriptor (not derivable), so both tests
  were repointed to `regex` (still unmigrated on this branch), preserving their
  exact assertions on a genuinely-unmigrated package. Evidence:
  `rg -n '"math\.abs"' src/builtins` before/after.
