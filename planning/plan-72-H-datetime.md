# plan-72-H: datetime package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/datetime.rs` (923 LOC, 11 metadata helpers, 1
`package_source_glue!`, 1 builtin-type helper, 3 custom-resolver helpers,
12 fixtures) to a `pub(crate) static DATETIME: BuiltinModule` with a
`DatetimeResolver` that preserves bug-349 named-argument overload binding,
arity-dependent implementation names, and time default padding.

References: plan-72 overview, `src/builtins/datetime.rs`,
`bugs/completed/bug-349-datetime-named-arg-misbinding.md`,
`bugs/completed/bug-173-builtins-syntaxcheck-typecheck-nits.md`,
`src/docs/spec/stdlib/02_datetime.md`,
`tests/{syntax,rt-behavior,byte-identity}/*/datetime/`.

## Goal

- `datetime::DATETIME` descriptor exists with `DatetimeResolver` covering
  `call_param_name_overloads` (line 190), `implementation_name(name, argc)`
  (line 363), and `default_argument_padding` (line 379).
- Legacy free functions become wrappers over `DATETIME`/`DatetimeResolver`.
- Parity tests cover every function, every named-argument overload variant,
  every arity-dependent implementation name, and the `time` default padding.

## Non-goals

- Do not broaden accepted overloads.
- Do not change generated internal helper names.
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/datetime.rs` is the highest-risk resolver package in this plan
because named-argument binding is correctness-sensitive per bug-349. It has 11
helpers, 1 builtin type (line 71), a `package_source_glue!` companion, and
three custom-resolver helpers. Fixture load is 12 projects.

## Phases

### Phase H1 — descriptor and resolver

- [ ] Add `pub(crate) static DATETIME: BuiltinModule` with every function,
      overload, parameter (canonical + aliases per overload), argument types,
      return type, implementation, and default.
- [ ] Add `BuiltinType` entry for the datetime builtin type.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Implement `DatetimeResolver` for parameter-name overloads,
      arity-dependent `implementation_name`, and `default_argument_padding`
      (time seconds/millis padding).
- [ ] Rewrite the 11 metadata helpers as wrappers over
      `DATETIME`/`DatetimeResolver`.
- [ ] Register `DATETIME` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: every `datetime.*` name, the bug-349 named-argument
      cases, `instant`/`duration`/`fixedOffset`/`parse` implementation names
      by arity, and `time` default padding.

Acceptance: `cargo test` passes and every `datetime.*` fixture runs clean
under `scripts/test-accept.sh target/debug/mfb target/accept-actual`,
including the existing `tests/byte-identity/datetime` cohort.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `datetime` fixtures per the overview.

## Corrections

Filled during execution.
