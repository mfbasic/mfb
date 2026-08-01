# plan-72-B: app package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/app.rs` (178 LOC, 8 metadata helpers, 1 `package_source_glue!`,
1 builtin-type helper, 0 custom-resolver helpers, 6 fixtures) to a
`pub(crate) static APP: BuiltinModule`.

References: plan-72 overview, `src/builtins/app.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/app/`.

## Goal

- `app::APP` descriptor exists and mirrors the current 8 helpers plus builtin
  type entries and `WhenImported` source injection.
- Legacy free functions in `app.rs` become wrappers that consult `APP`.
- Parity tests cover every `app.*` name, alias, return type, argument type,
  and the source companion type(s).

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.
- Do not touch other packages.

## Current State

`src/builtins/app.rs` uses `package_source_glue!` for its `.mfb` companion and
exposes exactly one builtin type helper. The `app` fixture load is 6 projects
(`find tests/{syntax,rt-behavior,byte-identity} -path '*/app/*/project.json' |
wc -l → 6`).

## Phases

### Phase B1 — descriptor and wrappers

- [ ] Add `pub(crate) static APP: BuiltinModule` with every function, overload,
      parameter (canonical + aliases), argument types, return type,
      implementation name, and default value.
- [ ] Add `BuiltinType` entries for the `app` builtin type(s).
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Rewrite `is_app_call`, `arity`, `resolve_call`, `expected_arguments`,
      `argument_types`, `call_param_names`, `call_return_type_name`, and the
      builtin-type helpers as wrappers over `APP`.
- [ ] Register `APP` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: assert `APP`-derived answers equal the pre-migration
      helpers for every name in `src/builtins/app.rs`.

Acceptance: `cargo test` passes and every `app.*` fixture in
`tests/{syntax,rt-behavior,byte-identity}` runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `app` fixtures per the overview.

## Corrections

Filled during execution.
