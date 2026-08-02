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

- [x] Add `pub(crate) static IO: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default. Done: 15 functions, single
      fixed-return overloads; `input`/`pollInput` model the optional trailing
      argument as `DefaultValue::Optional` (widens arity 0..1, no padding).
- [x] ~~Add `BuiltinType` entries for both io builtin types.~~ — moot: io has
      **no** builtin types. Its `is_builtin_type`/`builtin_type_fields` are
      constant `false`/`None` stubs (`io.rs` original lines 45–51), which the
      census miscounted as 2 "builtin-type helpers". `types: &[]`; the two
      wrappers now query the empty `IO.types` and still always report absence.
      See Corrections.
- [x] Rewrite the 8 metadata helpers as wrappers over `IO`.
      `is_io_call`/`arity`/`call_return_type_name`/`resolve_call` delegate to
      `DefaultResolver`; `is_builtin_type`/`builtin_type_fields` query
      `IO.types`; `call_param_names` (borrowed shape) and `expected_arguments`
      (bespoke `"no arguments"`) stay hand-authored statics pinned by parity.
- [x] Register `IO` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests for every `io.*` name (`parity_matches_descriptor`); the
      `expected_arguments` row is opted out (bespoke phrasing) and io has no
      builtin types to check.

Acceptance: `cargo test` passes; every `io.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: dd6e065b4

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `io` fixtures per the overview.

## Corrections

- **io has zero builtin types, not "both io builtin types".** The overview census
  (`btypes 2`) and this letter's Phase N1 both assumed io contributes two builtin
  types. It contributes none: `io.rs`'s original `is_builtin_type` returned a
  literal `false` and `builtin_type_fields` a literal `None` (they took `_name`).
  Evidence: original `src/builtins/io.rs:45-51`, and the metadata test
  `assert!(!is_builtin_type("Anything"))`. The census's `btypes` column counts
  *helper fns present*, not real types — for io those two fns are stubs. Landed
  with `types: &[]`; the wrappers now derive from the empty `IO.types` and remain
  always-absent, so behavior is byte-identical. No re-scope of other letters (the
  miscount is io-local; the two stub fns still count toward the plan's 209 helper
  population, which BB deletes).
