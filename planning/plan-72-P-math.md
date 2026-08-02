# plan-72-P: math package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/math.rs` (616 LOC, 6 metadata helpers, 0 source glue,
0 builtin types, 0 custom-resolver helpers, 45 fixtures) to a
`pub(crate) static MATH: BuiltinModule`.

References: plan-72 overview, `src/builtins/math.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/math/`,
`memory/clippy-fix-trims-double-double-constants.md`.

## Goal

- `math::MATH` descriptor exists and mirrors every current metadata helper.
- Legacy free functions in `math.rs` become wrappers over `MATH`.
- Parity tests cover every function name, arity, return type, and argument
  type.

## Non-goals

- Do not add builtin types or a source companion (math has neither today).
- Do not touch double-double constants (`[[clippy-fix-trims-double-double-constants]]`
  — do not run `clippy --fix` on `math.rs`).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/math.rs` is data-shaped with 6 descriptor-owned helpers, no
source companion, no builtin type, and no custom resolver. Fixture load is
45 projects.

## Phases

### Phase P1 — descriptor and wrappers

- [ ] Add `pub(crate) static MATH: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Rewrite the 6 metadata helpers as wrappers over `MATH`.
- [ ] Register `MATH` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `math.*` name and unknown-name behavior.

Acceptance: `cargo test` passes; every `math.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `math` fixtures per the overview.

## Corrections

Filled during execution.
