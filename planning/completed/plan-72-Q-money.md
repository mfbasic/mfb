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

- [x] Add `pub(crate) static MONEY: BuiltinModule` with every function,
      overload, parameter, argument types, return type, implementation
      (`Same` — inline lowering, no rewrite), and default.
- [x] Add `BuiltinType` entry for `Rounding` (`TypeKind::Enum`).
- [x] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [x] Rewrite the 8 metadata helpers as wrappers over `MONEY`
      (fully descriptor-derivable — `resolve_call`/`arity`/
      `call_return_type_name` delegate to `DefaultResolver`;
      `call_param_names`/`expected_arguments`/`argument_types` stay borrowed
      statics pinned by parity).
- [x] Register `MONEY` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests for every `money.*` name and the `Rounding` type.

Acceptance: `cargo test` passes; every `money.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: 96c7a03f4

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `money` fixtures per the overview.

## Corrections

- **money is fully descriptor-derivable (no hand-authored resolution).** Unlike
  json/net, money's every call has fixed positional argument types and a fixed
  return, so `resolve_call` delegates to `DefaultResolver::resolve_call` and
  `argument_types`/`expected_arguments` are pinned equal to the descriptor's
  per-position rendering. The only reason `call_param_names`/
  `expected_arguments`/`argument_types` stay hand-authored is that they return
  `&'static` borrowed shapes the owned `DefaultResolver` (yielding `Vec`/`String`)
  cannot produce; they are PINNED equal to `MONEY` by the parity test. `Rounding`
  is `TypeKind::Enum` (no record fields).
