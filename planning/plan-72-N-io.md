# plan-72-N: io package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/io.rs` (236 LOC, 8 metadata helpers, 0 source glue,
2 builtin-type helpers, 0 custom-resolver helpers, 31 fixtures) to a
`pub(crate) static IO: BuiltinModule`.

References: plan-72 overview, `src/builtins/io.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/io/`,
`memory/bug-pty-echo-race-under-coverage.md`.

## Goal

- `io::IO` descriptor exists and mirrors every current metadata helper
  plus both builtin-type entries.
- Legacy free functions in `io.rs` become wrappers over `IO`.
- Parity tests cover every function and both builtin types.

## Non-goals

- Do not add a source companion (io has none today).
- Do not remove the wrapper functions; `BB` owns deletion.
- Do not touch PTY echo timing (`[[bug-pty-echo-race-under-coverage]]`).

## Current State

`src/builtins/io.rs` has 8 descriptor-owned helpers and two builtin-type
entries; no source companion or custom resolver. Fixture load is 31
projects.

## Phases

### Phase N1 — descriptor and wrappers

- [ ] Add `pub(crate) static IO: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Add `BuiltinType` entries for both io builtin types.
- [ ] Rewrite the 8 metadata helpers as wrappers over `IO`.
- [ ] Register `IO` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `io.*` name and both builtin types.

Acceptance: `cargo test` passes; every `io.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `io` fixtures per the overview.

## Corrections

Filled during execution.
