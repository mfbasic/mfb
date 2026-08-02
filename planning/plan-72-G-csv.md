# plan-72-G: csv package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/csv.rs` (162 LOC, 7 metadata helpers, 1
`package_source_glue!`, 0 builtin types, 1 custom-resolver helper,
2 fixtures) to a `pub(crate) static CSV: BuiltinModule` with a light
resolver for `implementation_name(name)` (line 59).

References: plan-72 overview, `src/builtins/csv.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/csv/`.

## Goal

- `csv::CSV` descriptor exists with resolver support for `implementation_name`.
- Legacy free functions in `csv.rs` become wrappers over `CSV`.
- Parity tests cover every function and implementation-name case.

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/csv.rs` is small (162 LOC) with 7 metadata helpers, a
`package_source_glue!` companion, and a single non-typed
`implementation_name(name)` at line 59. Fixture load is 2 projects.

## Phases

### Phase G1 — descriptor and resolver

- [ ] Add `pub(crate) static CSV: BuiltinModule` with every function,
      overload, parameter, argument types, return type, implementation, and
      default.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Attach a resolver (or a static implementation-name table) for
      `implementation_name`.
- [ ] Rewrite the 7 metadata helpers as wrappers over `CSV`.
- [ ] Register `CSV` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `csv.*` name and every implementation-name
      case.

Acceptance: `cargo test` passes and `csv.*` fixtures run clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `csv` fixtures per the overview.

## Corrections

Filled during execution.
