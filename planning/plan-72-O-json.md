# plan-72-O: json package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/json.rs` (251 LOC, 8 metadata helpers, 1
`package_source_glue!`, 1 builtin-type helper, 1 custom-resolver helper,
8 fixtures) to a `pub(crate) static JSON: BuiltinModule`.

References: plan-72 overview, `src/builtins/json.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/json/`.

## Goal

- `json::JSON` descriptor exists with a resolver entry for
  `implementation_name(name)` (line 94).
- Legacy free functions in `json.rs` become wrappers over `JSON`.
- Parity tests cover every function, the builtin type, and every
  implementation-name case.

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/json.rs` has 8 helpers, one builtin type (line 17), a
`package_source_glue!` companion, and a non-typed `implementation_name(name)`
at line 94. Fixture load is 8 projects.

## Phases

### Phase O1 — descriptor and wrappers

- [ ] Add `pub(crate) static JSON: BuiltinModule` with every function,
      overload, parameter, argument types, return type, implementation, and
      default.
- [ ] Add `BuiltinType` entry for the json builtin type.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Attach a resolver (or a static implementation-name table) for
      `implementation_name`.
- [ ] Rewrite the 8 metadata helpers as wrappers over `JSON`.
- [ ] Register `JSON` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `json.*` name, the builtin type, and every
      implementation-name case.

Acceptance: `cargo test` passes; every `json.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `json` fixtures per the overview.

## Corrections

Filled during execution.
