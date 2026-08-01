# plan-72-Y: thread package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/thread.rs` (840 LOC, 7 metadata helpers, 0 source
glue, 1 builtin-type helper, 0 custom-resolver helpers, 0 fixtures) to a
`pub(crate) static THREAD: BuiltinModule`.

References: plan-72 overview, `src/builtins/thread.rs`,
`memory/thread-resource-plane-split.md`,
`memory/glibc-musl-thread-entry-alignment.md`.

## Goal

- `thread::THREAD` descriptor exists and mirrors every current metadata
  helper plus the builtin-type entry.
- Legacy free functions in `thread.rs` become wrappers over `THREAD`.
- Parity tests cover every function and the builtin type.

## Non-goals

- Do not change thread-resource plane split semantics
  (`[[thread-resource-plane-split]]`).
- Do not change trampoline alignment
  (`[[glibc-musl-thread-entry-alignment]]`).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/thread.rs` has 7 descriptor-owned helpers and one builtin
type. There are no fixtures under
`tests/{syntax,rt-behavior,byte-identity}/*/thread/`; parity is proven by
`cargo test` (unit + integration tests exercising the thread runtime).

## Phases

### Phase Y1 — descriptor and wrappers

- [ ] Add `pub(crate) static THREAD: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Add `BuiltinType` entry for the thread builtin type.
- [ ] Rewrite the 7 metadata helpers as wrappers over `THREAD`.
- [ ] Register `THREAD` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `thread.*` name and the builtin type.

Acceptance: `cargo test` passes. Because there are no thread fixtures
under `tests/`, acceptance for this letter is `cargo test` only unless a
new thread fixture is added.
Commit: —

## Validation

- `cargo test`

## Corrections

Filled during execution.
