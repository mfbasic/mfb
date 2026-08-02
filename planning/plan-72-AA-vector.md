# plan-72-AA: vector package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/vector.rs` (770 LOC, 7 metadata helpers, 1
`package_source_glue!`, 1 builtin-type helper, 1 custom-resolver helper,
23 fixtures) to a `pub(crate) static VECTOR: BuiltinModule` with a
`VectorResolver` that preserves typed `implementation_name`.

References: plan-72 overview, `src/builtins/vector.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/vector/`,
`memory/plan-32-execution-state.md` (RVV context — do not regress).

## Goal

- `vector::VECTOR` descriptor exists with `VectorResolver` covering
  `implementation_name(name, arg_types)` (line 248).
- Legacy free functions in `vector.rs` become wrappers over
  `VECTOR`/`VectorResolver`.
- Parity tests cover every function, the builtin type, and every
  arg-type driven implementation-name case.

## Non-goals

- Do not change RVV or dual-path codegen semantics
  (`[[plan-32-execution-state]]`).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/vector.rs` has 7 helpers, one builtin type (line 41), a
`package_source_glue!` companion, and a typed `implementation_name` at
line 248. Fixture load is 23 projects.

## Phases

### Phase AA1 — descriptor and resolver

- [ ] Add `pub(crate) static VECTOR: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Add `BuiltinType` entry for the vector builtin type.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Implement `VectorResolver` for typed `implementation_name`.
- [ ] Rewrite the 7 metadata helpers as wrappers over
      `VECTOR`/`VectorResolver`.
- [ ] Register `VECTOR` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: every `vector.*` name, the builtin type, and every
      typed implementation-name case.

Acceptance: `cargo test` passes; every `vector.*` fixture runs clean
under `scripts/test-accept.sh target/debug/mfb target/accept-actual`,
including the existing `tests/byte-identity/vector` cohort.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `vector` fixtures per the overview.

## Corrections

Filled during execution.
