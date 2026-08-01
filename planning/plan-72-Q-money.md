# plan-72-Q: money package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/money.rs` (189 LOC, 8 metadata helpers, 1
`package_source_glue!`, 1 builtin-type helper, 0 custom-resolver helpers,
6 fixtures) to a `pub(crate) static MONEY: BuiltinModule`.

References: plan-72 overview, `src/builtins/money.rs`,
`src/docs/spec/stdlib/13_money.md`,
`tests/{syntax,rt-behavior,byte-identity}/*/money/`.

## Goal

- `money::MONEY` descriptor exists and mirrors every current metadata
  helper plus the `Rounding` builtin type.
- Legacy free functions in `money.rs` become wrappers over `MONEY`.
- Parity tests cover every function and the builtin type.

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/money.rs` has 8 helpers, one builtin type (`Rounding`), and a
`package_source_glue!` companion. Fixture load is 6 projects.

## Phases

### Phase Q1 — descriptor and wrappers

- [ ] Add `pub(crate) static MONEY: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Add `BuiltinType` entry for `Rounding`.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Rewrite the 8 metadata helpers as wrappers over `MONEY`.
- [ ] Register `MONEY` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `money.*` name and the `Rounding` type.

Acceptance: `cargo test` passes; every `money.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `money` fixtures per the overview.

## Corrections

Filled during execution.
