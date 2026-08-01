# plan-72-A: descriptor core and compatibility wrappers

Last updated: 2026-07-31
Overall Effort: huge (> 3d)
Effort: medium (1h-2h)
Depends on: nothing; re-run plan-72 prerequisites first.

This sub-plan adds the descriptor vocabulary and registry lookup API without
changing any package behavior. It lands first because every later migration
depends on a tested compatibility layer.

References: plan-72 overview, `.ai/compiler.md`, `src/builtins/mod.rs`,
`src/syntaxcheck/builtins.rs`.

## Goal

- A `BuiltinModule` descriptor model exists and can answer the same metadata
  questions as the current five-package helper functions.
- The descriptor API is covered by unit tests using a small test module and by
  adapter tests against at least one real package.

## Non-goals

- Do not migrate any target package table yet except for minimal test fixtures.
- Do not change external aggregate helper behavior.
- Do not remove existing free functions.

## Current State

`src/syntaxcheck/builtins.rs:BuiltinPackage` proves value-level package dispatch
is already acceptable locally, but it is private to syntax checking and stores
function pointers to package free functions. `src/builtins/mod.rs` has aggregate
functions that can become the compatibility bridge.

## Phases

### Phase A1 — descriptor vocabulary

- [ ] Add `src/builtins/descriptor.rs` and wire `mod descriptor; pub(crate) use`
      exports from `src/builtins/mod.rs`.
- [ ] Define `BuiltinModule`, `BuiltinFunction`, `BuiltinOverload`, `Parameter`,
      `ParameterType`, `ReturnType`, `DefaultValue`, `Implementation`,
      `Lowering`, `BuiltinFlags`, `BuiltinType`, `TypeKind`, `BuiltinSource`,
      `InjectionRule`, and `BuiltinResolver`.
- [ ] Provide `DefaultResolver` methods for data-only modules: contains,
      arity, parameter names, argument type list, fixed return type, expected
      argument rendering, implementation name, and default padding.
- [ ] Tests: add focused `#[cfg(test)]` tests in `descriptor.rs` for aliases,
      min/max arity, fixed return resolution, default rendering, and unresolved
      calls.

Acceptance: `cargo test` passes, and descriptor tests prove the API can derive
the metadata currently split across `arity`, `expected_arguments`,
`argument_types`, `call_param_names`, and `call_return_type_name`.
Commit: —

### Phase A2 — registry shell

- [ ] Add `BuiltinRegistry` as a deterministic static-slice wrapper; lookup by
      module name and by fully qualified function name.
- [ ] Add a test-only descriptor module and a test-only registry that exercises
      lookup order, unknown modules, unknown functions, and duplicate-name guard
      behavior.
- [ ] Add adapter helper functions in `src/builtins/mod.rs` that can query
      registry descriptors but currently fall back to existing package helpers.

Acceptance: `cargo test` passes; no production package behavior is routed
through descriptors yet.
Commit: —

### Phase A3 — parity harness

- [ ] Add a reusable parity test helper that compares a descriptor-backed module
      against a legacy helper set for: membership, arity, parameter names,
      parameter-name overloads, return type, expected arguments, argument types,
      implementation name, defaults, and builtin type fields where applicable.
- [ ] Make the helper support package-specific resolver callbacks so D can reuse
      it for datetime and encoding.
- [ ] Document in comments that parity tests are the migration gate and must be
      deleted only after the legacy helpers are gone in E.

Acceptance: `cargo test` passes and the parity helper is used by at least one
test-only descriptor.
Commit: —

## Validation

- `cargo test`
- No acceptance or byte-identity run is required for A if no production dispatch
  changes; if any aggregate helper begins consulting descriptors, run the full
  plan validation commands from the overview.

## Corrections

Filled during execution.
