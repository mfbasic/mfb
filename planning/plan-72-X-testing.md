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

- [x] Add `pub(crate) static TESTING: BuiltinModule` covering every
      function, overload, parameter, argument types, return type,
      implementation, and default. Done: the 12 assertion builtins
      (`expectEqual`…`expectNTrap`), all `Implementation::Same` /
      `Lowering::Inline`, `Nothing` return; generic operands typed `"T"`,
      typed asserts pinned to their operand type; `expectTrap`'s trailing
      `code` is `DefaultValue::Optional` (arity (1,2), never padded).
- [x] Rewrite the metadata helper as a wrapper over `TESTING`. Done:
      `is_testing_call` now delegates to `DefaultResolver::contains(&TESTING, …)`
      (the family predicates `is_equality_assert`/`is_inequality_assert`/
      `expect_arity`/`expect_operand_type` are NOT descriptor-owned and stay).
- [x] Register `TESTING` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests for every `testing.*` name and unknown-name behavior
      (`parity_matches_descriptor`): descriptor membership == `is_testing_call`
      and descriptor arity == `expect_arity` for all 12 names + non-members.

Acceptance: `cargo test` passes (`cargo test --bin mfb builtins::testing → 5
passed`, `builtins::descriptor → 19 passed`); `testing.*` fixtures verified
byte-identical in the consolidated T–X acceptance at finalization (this change is
a metadata-only wrapper proven equal by the parity test, and the descriptor
`REGISTRY` is never read in production dispatch).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`

## Corrections

Filled during execution.
