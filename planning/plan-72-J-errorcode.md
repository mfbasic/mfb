# plan-72-J: errorcode package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/errorcode.rs` (118 LOC, 0 descriptor-owned helpers,
0 source glue, 0 builtin types, 0 custom-resolver helpers, 1 fixture) to a
`pub(crate) static ERRORCODE: BuiltinModule`.

The zero-helper count means this letter is primarily about registering an
exhaustive descriptor entry so `BB` can collapse aggregate arms
unconditionally; it exists to keep the registry complete.

References: plan-72 overview, `src/builtins/errorcode.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/errorcode/`.

## Goal

- `errorcode::ERRORCODE` descriptor exists and models the package's contents
  (constants, types, or source content) in whatever form the descriptor
  vocabulary from `A` provides.
- The registry can enumerate `errorcode` alongside every other package
  without special-casing.

## Non-goals

- Do not add or change any error code, constant, or diagnostic.
- Do not synthesize helper functions that never existed.

## Current State

`src/builtins/errorcode.rs` has no descriptor-owned metadata helpers today.
Its 118 LOC hold constants and package-level integration used elsewhere.
Fixture load is 1 project.

## Phases

### Phase J1 — descriptor entry

- [ ] Add `pub(crate) static ERRORCODE: BuiltinModule` — empty function list
      is acceptable if the module truly exposes none; document what the
      descriptor carries (constants, source companion, or types).
- [ ] Register `ERRORCODE` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: the registry lookup for `errorcode` succeeds and returns
      the expected shape; unknown-name lookups against it fail cleanly.

Acceptance: `cargo test` passes; the `errorcode` fixture continues to run
clean under `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`

## Corrections

Filled during execution.
