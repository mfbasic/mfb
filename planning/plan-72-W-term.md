# plan-72-W: term package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/term.rs` (477 LOC, 9 metadata helpers, 1
`package_source_glue!`, 3 builtin-type helpers, 0 custom-resolver helpers,
33 fixtures) to a `pub(crate) static TERM: BuiltinModule`.

References: plan-72 overview, `src/builtins/term.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/term/`,
`memory/plan-62-presentation-mode-system.md`.

## Goal

- `term::TERM` descriptor exists and mirrors every current metadata helper
  plus all four builtin types (`TermColor`, `TermSize`, `LineStyle`,
  `FillStyle`), with `TermColor` and `TermSize` record fields preserved.
- Legacy free functions in `term.rs` become wrappers over `TERM`.
- Parity tests cover every function, every type, and every record field.

## Non-goals

- Do not change presentation-mode semantics
  (`[[plan-62-presentation-mode-system]]`).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/term.rs` has 9 descriptor-owned helpers and three
builtin-type helpers (`is_builtin_type`, `builtin_type_fields`,
`param_types`) covering both the hard-coded types (`TermColor`,
`TermSize`) and the source companion types (`LineStyle`, `FillStyle`). It
injects a companion via `package_source_glue!`. Fixture load is 33
projects.

## Phases

### Phase W1 — descriptor and wrappers

- [ ] Add `pub(crate) static TERM: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Add `BuiltinType` entries for `TermColor`, `TermSize`, `LineStyle`,
      and `FillStyle`. `TermColor` and `TermSize` must preserve their
      record fields; `LineStyle` and `FillStyle` come from the source
      companion.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Rewrite `is_term_call`, `is_builtin_type`, `builtin_type_fields`,
      `call_param_names`, `param_types`, `call_return_type_name`,
      `resolve_call`, `expected_arguments`, and `arity` as wrappers over
      `TERM`.
- [ ] Register `TERM` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: every `term.*` name, all four builtin types, and every
      record field.

Acceptance: `cargo test` passes; every `term.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`, including
the existing `tests/byte-identity/term` cohort.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `term` fixtures per the overview.

## Corrections

Filled during execution.
