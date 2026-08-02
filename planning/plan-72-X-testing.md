# plan-72-X: testing package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/testing.rs` (175 LOC, 1 metadata helper, 0 source
glue, 0 builtin types, 0 custom-resolver helpers, 8 fixtures) to a
`pub(crate) static TESTING: BuiltinModule`.

References: plan-72 overview, `src/builtins/testing.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/testing/`.

## Goal

- `testing::TESTING` descriptor exists and mirrors the single current
  helper along with whatever functions/types the module exposes.
- Legacy free functions in `testing.rs` become wrappers over `TESTING`.
- Parity tests cover the module surface.

## Non-goals

- Do not add builtin types or a source companion (testing has neither).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/testing.rs` exposes exactly one descriptor-owned helper.
Fixture load is 8 projects.

## Phases

### Phase X1 — descriptor and wrapper

- [ ] Add `pub(crate) static TESTING: BuiltinModule` covering every
      function, overload, parameter, argument types, return type,
      implementation, and default.
- [ ] Rewrite the metadata helper as a wrapper over `TESTING`.
- [ ] Register `TESTING` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `testing.*` name and unknown-name behavior.

Acceptance: `cargo test` passes; every `testing.*` fixture runs clean
under `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`

## Corrections

Filled during execution.
