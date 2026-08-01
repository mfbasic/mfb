# plan-72-L: general package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/general.rs` (815 LOC, 6 metadata helpers, 0 source
glue, 0 builtin types, 0 custom-resolver helpers, 26 fixtures) to a
`pub(crate) static GENERAL: BuiltinModule`.

References: plan-72 overview, `src/builtins/general.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/general/`.

## Goal

- `general::GENERAL` descriptor exists and mirrors every current metadata
  helper.
- Legacy free functions in `general.rs` become wrappers over `GENERAL`.
- Parity tests cover every function name, arity, return type, and argument
  type.

## Non-goals

- Do not add builtin types or a source companion (general has neither
  today).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/general.rs` is a large but data-shaped module with 6
descriptor-owned helpers and no source, type, or resolver customization.
Fixture load is 26 projects.

## Phases

### Phase L1 — descriptor and wrappers

- [ ] Add `pub(crate) static GENERAL: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Rewrite the 6 metadata helpers as wrappers over `GENERAL`.
- [ ] Register `GENERAL` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `general.*` name and unknown-name behavior.

Acceptance: `cargo test` passes; every `general.*` fixture runs clean
under `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `general` fixtures per the overview.

## Corrections

Filled during execution.
