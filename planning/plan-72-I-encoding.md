# plan-72-I: encoding package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/encoding.rs` (596 LOC, 10 metadata helpers, 1
`package_source_glue!`, 0 builtin types, 3 custom-resolver helpers,
29 fixtures) to a `pub(crate) static ENCODING: BuiltinModule` with an
`EncodingResolver` that preserves `utf8Encode` / `utf8Decode` overload
selection and monomorph overload target resolution.

References: plan-72 overview, `src/builtins/encoding.rs`,
`planning/completed/plan-02-encoding.md`,
`tests/{syntax,rt-behavior,byte-identity}/*/encoding/`,
`src/monomorph/lower.rs`.

## Goal

- `encoding::ENCODING` descriptor exists with `EncodingResolver` covering
  `implementation_name` (line 210), `resolve_overload_target` (line 252),
  and `is_overloaded` (line 271).
- Legacy free functions become wrappers over
  `ENCODING`/`EncodingResolver`.
- Parity tests cover every function, every overload target, and every
  invalid-overload diagnostic path.

## Non-goals

- Do not broaden accepted overloads.
- Do not change generated internal helper names.
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/encoding.rs` has 10 helpers, a `package_source_glue!`
companion, and three custom-resolver helpers driving typed
`utf8Encode`/`utf8Decode` overload behavior. Fixture load is 29 projects —
the second-largest custom-resolver cohort.

## Phases

### Phase I1 — descriptor and resolver

- [ ] Add `pub(crate) static ENCODING: BuiltinModule` with every function,
      overload, parameter, argument types, return type, implementation, and
      default.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Implement `EncodingResolver` for `implementation_name`,
      `is_overloaded`, and `resolve_overload_target`.
- [ ] Rewrite the 10 metadata helpers as wrappers over
      `ENCODING`/`EncodingResolver`.
- [ ] Register `ENCODING` with the `BuiltinRegistry` from plan-72-A.
- [ ] Route `src/monomorph/lower.rs` encoding overload target resolution
      through the descriptor API in this letter (do not defer to `BB`).
- [ ] Parity tests: every `encoding.*` name, ambiguous `utf8Encode` invalid
      fixture behavior, `bytes`/`ints` overload targets, and implementation
      names.

Acceptance: `cargo test` passes; every `encoding.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`, including the
existing `tests/byte-identity/encoding` cohort.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `encoding` fixtures per the overview.

## Corrections

Filled during execution.
