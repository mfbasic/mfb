# plan-72-M: http package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/http.rs` (581 LOC, 9 metadata helpers, 1
`package_source_glue!`, 1 builtin-type helper, 2 custom-resolver helpers,
7 fixtures) to a `pub(crate) static HTTP: BuiltinModule` with an
`HttpResolver` that preserves `default_argument_padding` and typed
`implementation_name`.

References: plan-72 overview, `src/builtins/http.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/http/`.

## Goal

- `http::HTTP` descriptor exists with `HttpResolver` covering
  `default_argument_padding` (line 241) and `implementation_name(name,
  arg_types)` (line 277).
- Legacy free functions become wrappers over `HTTP`/`HttpResolver`.
- Parity tests cover every function, the builtin type, every default padding
  slot, and every arg-type driven implementation-name case.

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.
- Do not change runtime helper selection semantics.

## Current State

`src/builtins/http.rs` has 9 helpers, one builtin type (line 76), a
`package_source_glue!` companion, and two custom-resolver helpers. Fixture
load is 7 projects.

## Phases

### Phase M1 — descriptor and resolver

- [ ] Add `pub(crate) static HTTP: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Add `BuiltinType` entry for the http builtin type.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Implement `HttpResolver` for `default_argument_padding` and typed
      `implementation_name`.
- [ ] Rewrite the 9 metadata helpers as wrappers over
      `HTTP`/`HttpResolver`.
- [ ] Register `HTTP` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: every `http.*` name, every default-padding slot, every
      typed implementation-name case, and the builtin type.

Acceptance: `cargo test` passes; every `http.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`, including
the existing `tests/byte-identity/http` cohort.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `http` fixtures per the overview.

## Corrections

Filled during execution.
