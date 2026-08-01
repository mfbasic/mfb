# plan-72-A: descriptor core and compatibility wrappers

Last updated: 2026-08-01
Overall Effort: huge (> 3d)
Effort: medium (1h-2h)
Depends on: nothing; re-run plan-72 prerequisites first.

This sub-plan adds the descriptor vocabulary and registry lookup API without
changing any package behavior. It lands first because every later per-package
letter (B through AA) depends on a tested compatibility layer that can serve
all 26 builtin modules.

References: plan-72 overview, `.ai/compiler.md`, `src/builtins/mod.rs`,
`src/syntaxcheck/builtins.rs`.

## Goal

- A `BuiltinModule` descriptor model exists and can answer the same metadata
  questions as the current per-package helper functions across all 26
  packages.
- The descriptor API is covered by unit tests using a small test module and by
  adapter tests against at least one real package.

## Non-goals

- Do not migrate any real package table yet except for minimal test fixtures.
- Do not change external aggregate helper behavior.
- Do not remove existing free functions.
- Do not tune the descriptor API for one package's shape at the expense of the
  others; the vocabulary must serve every one of the 26 packages listed in the
  overview.

## Current State

`src/syntaxcheck/builtins.rs:BuiltinPackage` proves value-level package dispatch
is already acceptable locally, but it is private to syntax checking and stores
function pointers to package free functions. `src/builtins/mod.rs` has aggregate
functions that can become the compatibility bridge.

## Phases

### Phase A1 — descriptor vocabulary

- [x] Add `src/builtins/descriptor.rs` and wire `mod descriptor;` into
      `src/builtins/mod.rs` (placed in alphabetical position after `datetime`).
      No `pub(crate) use` re-export needed: consumers reach items via
      `builtins::descriptor::*` and none is used outside the module yet.
- [x] Define `BuiltinModule`, `BuiltinFunction`, `BuiltinOverload`, `Parameter`,
      `ParameterType`, `ReturnType`, `DefaultValue`, `Implementation`,
      `Lowering`, `BuiltinFlags`, `BuiltinType`, `TypeKind`, `BuiltinSource`,
      `InjectionRule`, and `BuiltinResolver`.
- [x] Provide `DefaultResolver` methods for data-only modules: contains,
      arity, parameter names, argument type list, fixed return type, expected
      argument rendering, implementation name, and default padding.
- [x] Tests: add focused `#[cfg(test)]` tests in `descriptor.rs` for aliases,
      min/max arity, fixed return resolution, default rendering, and unresolved
      calls (9 tests, all passing).

Acceptance: `cargo test` passes, and descriptor tests prove the API can derive
the metadata currently split across `arity`, `expected_arguments`,
`argument_types`, `call_param_names`, and `call_return_type_name`.
Commit: 4bdcc0e89

### Phase A2 — registry shell

- [x] Add `BuiltinRegistry` as a deterministic static-slice wrapper; lookup by
      module name and by fully qualified function name (`descriptor.rs`), plus
      `duplicate_module_name`/`duplicate_function_name` guards and an empty
      production `REGISTRY` static (letters B..AA append their `&<PKG>`).
- [x] Add a test-only descriptor module and a test-only registry that exercises
      lookup order, unknown modules, unknown functions, and duplicate-name guard
      behavior (7 registry tests, incl. a colliding registry).
- [x] Add adapter helper functions in `src/builtins/mod.rs`
      (`registry_is_call`/`registry_arity`/`registry_return_type_name`/
      `registry_expected_arguments`) that query registry descriptors but fall
      back to existing package helpers; empty production registry ⇒ fallback ⇒
      byte-identical to legacy. 2 adapter tests prove both branches.

Acceptance: `cargo test` passes; no production package behavior is routed
through descriptors yet (production `REGISTRY` is empty; adapters unused until
letter B wires them).
Commit: —

### Phase A3 — parity harness

- [ ] Add a reusable parity test helper that compares a descriptor-backed module
      against a legacy helper set for: membership, arity, parameter names,
      parameter-name overloads, return type, expected arguments, argument types,
      implementation name, defaults, and builtin type fields where applicable.
- [ ] Make the helper support package-specific resolver callbacks so the
      custom-resolver letters (`H` datetime, `I` encoding, and any other letter
      whose census `custom` column is nonzero) can reuse it.
- [ ] Document in comments that parity tests are the migration gate and must be
      deleted only after the legacy helpers are gone in `BB`.

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
