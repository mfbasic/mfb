# plan-72-U: resource package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/resource.rs` (364 LOC, 0 descriptor-owned helpers,
0 source glue, 0 builtin types, 0 custom-resolver helpers, 0 fixtures) to
a `pub(crate) static RESOURCE: BuiltinModule`.

Like `errorcode`, this letter is documentation-scale — it exists so the
registry can enumerate `resource` alongside every other package without
special-casing. It also enforces `[[res-is-a-pointer-not-a-borrow]]`
vocabulary in any prose added around the descriptor entry.

References: plan-72 overview, `src/builtins/resource.rs`,
`memory/res-is-a-pointer-not-a-borrow.md`.

## Goal

- `resource::RESOURCE` descriptor exists and models the package's contents
  (constants, types, or lifecycle glue) in whatever form the descriptor
  vocabulary from `A` provides.
- The registry can enumerate `resource` alongside every other package
  without special-casing.

## Non-goals

- Do not add or change any resource lifecycle semantics.
- Do not use Rust ownership vocabulary in the descriptor prose
  (`[[res-is-a-pointer-not-a-borrow]]`).
- Do not synthesize helper functions that never existed.

## Current State

`src/builtins/resource.rs` has no descriptor-owned metadata helpers today
and no test fixtures under `tests/{syntax,rt-behavior,byte-identity}/*/resource/`.
Its 364 LOC hold lifecycle glue used elsewhere.

## Phases

### Phase U1 — descriptor entry

- [x] Add `pub(crate) static RESOURCE: BuiltinModule` — empty function list
      is acceptable if the module truly exposes none; document what the
      descriptor carries. Done: empty `functions`/`types`, no `source`/`resolver`;
      the doc comment records that a resource value is a POINTER
      ([[res-is-a-pointer-not-a-borrow]]) and that the `ResourceRegistry` table,
      not the descriptor, stays the close-op authority.
- [x] Register `RESOURCE` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests: registry lookup for `resource` succeeds; unknown-name
      lookups against it fail cleanly (`descriptor_is_registered_and_empty`).

Acceptance: `cargo test` passes (`cargo test --bin mfb builtins::resource → 6
passed`).
Commit: 89f34e816

## Validation

- `cargo test`

## Corrections

Filled during execution.
