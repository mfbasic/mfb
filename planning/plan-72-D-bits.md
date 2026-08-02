# plan-72-D: bits package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/bits.rs` (237 LOC, 6 metadata helpers, 0 source glue,
0 builtin types, 0 custom-resolver helpers, 18 fixtures) to a
`pub(crate) static BITS: BuiltinModule`. Pure data-shaped module.

References: plan-72 overview, `src/builtins/bits.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/bits/`.

## Goal

- `bits::BITS` descriptor exists and answers every `bits.*` metadata query
  currently served by the 6 free-function helpers.
- Legacy free functions in `bits.rs` become wrappers that consult `BITS`.
- Parity tests cover every function name, arity, return type, and argument
  type.

## Non-goals

- Do not add builtin types or a source companion (bits has neither today).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/bits.rs` is a fully data-shaped module with 6 descriptor-owned
helpers and no source companion, custom resolver, or builtin type. Fixture
load is 18 projects.

## Phases

### Phase D1 — descriptor and wrappers

- [x] Add `pub(crate) static BITS: BuiltinModule` — 17 functions, each a single
      fixed-arity overload of `Integer` params → `Integer`, `Implementation::Same`,
      `Lowering::Inline`; no types/source/resolver.
- [x] Rewrite the 6 metadata helpers as wrappers over `BITS`:
      `is_bits_call`/`arity`/`call_return_type_name`/`resolve_call` delegate to
      `DefaultResolver`; `call_param_names`/`expected_arguments` stay static
      (borrowed `&'static` ABI the owned `DefaultResolver` cannot produce), pinned
      equal to `BITS` by the parity test (same pattern as B). See Corrections.
- [x] Register `BITS` with the `BuiltinRegistry` (`descriptor::REGISTRY` now
      `new(&[&app::APP, &bits::BITS])`); updated A's registry test and the mod.rs
      adapter fallback test (now uses unmigrated `math`).
- [x] Parity tests: `parity_matches_descriptor` covers every `bits.*` name +
      `bits.nope`, plus `resolve_call` accept/reject.

Acceptance: `cargo test` passes and every `bits.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual` (19 fixtures pass;
byte-identity via the combined D+E artifact-gate at finalization).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `bits` fixtures per the overview.

## Corrections

- **Two helpers stay static, pinned by parity (same as plan-72-B).**
  `call_param_names` and `expected_arguments` return `&'static` borrowed shapes
  the owned `DefaultResolver` (`Vec`/`String`) cannot be coerced to, and their
  consumers require the borrow. They remain static literals held equal to `BITS`
  by `parity_matches_descriptor`; BB deletes them. The other 4 helpers delegate.
- **`is_bits_shift` left untouched.** It is a bits-specific fallibility predicate,
  not one of the 6 descriptor-owned metadata helpers, so it is out of D's scope.
- **Updated cross-cutting tests A left behind.** Registering `bits` flipped
  `descriptor::tests::production_registry_holds_migrated_packages` (now asserts
  bits present, `math` absent) and the mod.rs adapter fallback test (renamed to
  `adapters_fall_back_for_unmigrated_packages`, uses `math`; added
  `adapters_use_descriptor_for_migrated_bits`). These were pinned to the pre-D
  registry contents.
