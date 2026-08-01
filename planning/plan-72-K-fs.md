# plan-72-K: fs package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/fs.rs` (713 LOC, 7 metadata helpers, 0 source glue,
1 builtin-type helper, 0 custom-resolver helpers, 98 fixtures) to a
`pub(crate) static FS: BuiltinModule`. Effort is large because the fixture
cohort (98 projects) is the largest in this plan.

References: plan-72 overview, `src/builtins/fs.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/fs/`.

## Goal

- `fs::FS` descriptor exists and mirrors every current metadata helper and
  the builtin-type entry.
- Legacy free functions in `fs.rs` become wrappers over `FS`.
- Parity tests cover every function and the builtin type; acceptance covers
  the 98-fixture cohort.

## Non-goals

- Do not add a source companion (fs has none today).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/fs.rs` has 7 helpers and one builtin type; no `package_source_glue!`
and no custom resolver. Its fixture load is 98 projects — the acceptance
window on this letter is the largest per-package run in the plan.

## Phases

### Phase K1 — descriptor and wrappers

- [ ] Add `pub(crate) static FS: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Add `BuiltinType` entry for the fs builtin type.
- [ ] Rewrite the 7 metadata helpers as wrappers over `FS`.
- [ ] Register `FS` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: every `fs.*` name and the builtin type.

Acceptance: `cargo test` passes; every `fs.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `fs` fixtures per the overview.

## Corrections

Filled during execution.
