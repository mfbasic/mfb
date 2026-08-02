# plan-72-S: os package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/os.rs` (274 LOC, 6 metadata helpers, 0 source glue,
0 builtin types, 0 custom-resolver helpers, 35 fixtures) to a
`pub(crate) static OS: BuiltinModule`.

References: plan-72 overview, `src/builtins/os.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/os/`.

## Goal

- `os::OS` descriptor exists and mirrors every current metadata helper.
- Legacy free functions in `os.rs` become wrappers over `OS`.
- Parity tests cover every function name, arity, return type, and argument
  type.

## Non-goals

- Do not add builtin types or a source companion (os has neither today).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/os.rs` is data-shaped with 6 descriptor-owned helpers and no
source companion, builtin type, or custom resolver. Fixture load is 35
projects.

## Phases

### Phase S1 — descriptor and wrappers

- [ ] Add `pub(crate) static OS: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Rewrite the 6 metadata helpers as wrappers over `OS`.
- [ ] Register `OS` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `os.*` name and unknown-name behavior.

Acceptance: `cargo test` passes; every `os.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `os` fixtures per the overview.

## Corrections

Filled during execution.
