# plan-72-D: bits package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/bits.rs` (237 LOC, 6 metadata helpers, 0 source glue,
0 builtin types, 0 custom-resolver helpers, 18 fixtures) to a
`pub(crate) static BITS: BuiltinModule`. Pure data-shaped module.

References: plan-72 overview, `src/builtins/bits.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/bits/`.

## Goal

- `bits::BITS` descriptor exists and answers every `bits.*` metadata query
  currently served by the 6 free-function helpers.
- Legacy free functions in `bits.rs` become wrappers that consult `BITS`.
- Parity tests cover every function name, arity, return type, and argument
  type.

## Non-goals

- Do not add builtin types or a source companion (bits has neither today).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/bits.rs` is a fully data-shaped module with 6 descriptor-owned
helpers and no source companion, custom resolver, or builtin type. Fixture
load is 18 projects.

## Phases

### Phase D1 — descriptor and wrappers

- [ ] Add `pub(crate) static BITS: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return type,
      implementation name, and default value.
- [ ] Rewrite the 6 metadata helpers as wrappers over `BITS`.
- [ ] Register `BITS` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `bits.*` name and unknown-name behavior.

Acceptance: `cargo test` passes and every `bits.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `bits` fixtures per the overview.

## Corrections

Filled during execution.
