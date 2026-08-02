# plan-72-E: collections package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/collections.rs` (1355 LOC, 10 metadata helpers, 0
source glue, 0 builtin types, 0 custom-resolver helpers, 50 fixtures) to a
`pub(crate) static COLLECTIONS: BuiltinModule`. Effort is large because of
LOC and fixture blast radius, not because of custom behavior.

References: plan-72 overview, `src/builtins/collections.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/collections/`,
`memory/collection-memory-mgmt.md`.

## Goal

- `collections::COLLECTIONS` descriptor exists and mirrors every current
  metadata helper.
- Legacy free functions become wrappers over `COLLECTIONS`.
- Parity tests cover every function, including MUT-append and shrink-related
  overloads whose behavior is load-bearing per the collection memory notes.

## Non-goals

- Do not change collection memory semantics (`[[collection-memory-mgmt]]`).
- Do not remove the wrapper functions; `BB` owns deletion.
- Do not merge with `general`.

## Current State

`src/builtins/collections.rs` is the largest data-shaped builtin at 1355 LOC
with 10 descriptor-owned helpers and no source companion or custom resolver.
Fixture load is 50 projects — the largest cohort in this plan after `fs`.

## Phases

### Phase E1 — descriptor and wrappers

- [ ] Add `pub(crate) static COLLECTIONS: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return type,
      implementation name, and default value.
- [ ] Rewrite the 10 metadata helpers as wrappers over `COLLECTIONS`.
- [ ] Register `COLLECTIONS` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `collections.*` name, alias, and unknown-name
      behavior.

Acceptance: `cargo test` passes; every `collections.*` fixture across
`tests/{syntax,rt-behavior,byte-identity}` runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `collections` fixtures per the overview.

## Corrections

Filled during execution.
