# plan-72-G: csv package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/csv.rs` (162 LOC, 7 metadata helpers, 1
`package_source_glue!`, 0 builtin types, 1 custom-resolver helper,
2 fixtures) to a `pub(crate) static CSV: BuiltinModule` with a light
resolver for `implementation_name(name)` (line 59).

References: plan-72 overview, `src/builtins/csv.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/csv/`.

## Goal

- `csv::CSV` descriptor exists with resolver support for `implementation_name`.
- Legacy free functions in `csv.rs` become wrappers over `CSV`.
- Parity tests cover every function and implementation-name case.

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/csv.rs` is small (162 LOC) with 7 metadata helpers, a
`package_source_glue!` companion, and a single non-typed
`implementation_name(name)` at line 59. Fixture load is 2 projects.

## Phases

### Phase G1 — descriptor and resolver

- [x] Add `pub(crate) static CSV: BuiltinModule` — `parse(value/text: String)`
      → `List OF List OF String`, `stringify(value: List OF List OF String)` →
      `String`; `Fixed` returns; `Implementation::Rewrite(__csv_parse/__csv_stringify)`.
- [x] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, loader: source_file }`.
- [x] ~~Attach a resolver~~ — moot: `implementation_name(name)` is a fixed
      name→symbol map = `Implementation::Rewrite`, so a **static table** (no
      resolver) suffices. The plan's "1 custom-resolver helper" overstated it.
- [x] Rewrite the metadata helpers over `CSV`: `is_csv_call`/`arity`/
      `call_return_type_name`/`resolve_call`/`implementation_name` delegate to
      `DefaultResolver`; `call_param_names`/`expected_arguments` stay static
      (borrowed ABI), pinned by parity.
- [x] Register `CSV` with the `BuiltinRegistry`
      (`new(&[&app::APP, &bits::BITS, &collections::COLLECTIONS, &csv::CSV])`).
- [x] Parity test `parity_matches_descriptor`: both members + `csv.other`
      (membership, arity, param names, return type, expected args, impl name),
      plus resolve_call/rewrite.

Acceptance: `cargo test` passes and `csv.*` fixtures run clean under
`scripts/test-accept.sh` (2 fixtures; byte-identity via the combined C/F/G/H/I
artifact-gate at finalization).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `csv` fixtures per the overview.

## Corrections

Filled during execution.
