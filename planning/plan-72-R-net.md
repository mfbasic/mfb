# plan-72-R: net package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/net.rs` (725 LOC, 11 metadata helpers, 1
`package_source_glue!`, 2 builtin-type helpers, 2 custom-resolver helpers,
46 fixtures) to a `pub(crate) static NET: BuiltinModule` with a
`NetResolver` that preserves parameter-name overloads and typed
implementation-name selection.

References: plan-72 overview, `src/builtins/net.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/net/`,
`memory/thread-resource-plane-split.md`.

## Goal

- `net::NET` descriptor exists with `NetResolver` covering
  `call_param_name_overloads` (line 135) and `implementation_name(name)`
  (line 324).
- Legacy free functions become wrappers over `NET`/`NetResolver`.
- Parity tests cover every function, both builtin types, every
  parameter-name overload, and every implementation-name case.

## Non-goals

- Do not change the thread-resource plane split
  (`[[thread-resource-plane-split]]`).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/net.rs` has 11 helpers, two builtin types (lines 74 and 87),
a `package_source_glue!` companion, and two custom-resolver helpers.
Fixture load is 46 projects — the largest custom-resolver cohort in the
plan.

## Phases

### Phase R1 — descriptor and resolver

- [ ] Add `pub(crate) static NET: BuiltinModule` with every function,
      overload, parameter (canonical + aliases per overload), argument
      types, return type, implementation, and default.
- [ ] Add `BuiltinType` entries for both net builtin types with record
      fields preserved.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Implement `NetResolver` for `call_param_name_overloads` and
      `implementation_name`.
- [ ] Rewrite the 11 metadata helpers as wrappers over
      `NET`/`NetResolver`.
- [ ] Register `NET` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: every `net.*` name, both builtin types, every
      parameter-name overload, and every implementation-name case.

Acceptance: `cargo test` passes; every `net.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`, including
the existing `tests/byte-identity/net` cohort.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `net` fixtures per the overview.

## Corrections

Filled during execution.
