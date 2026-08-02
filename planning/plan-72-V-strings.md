# plan-72-V: strings package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/strings.rs` (875 LOC, 10 metadata helpers, 0
`package_source_glue!` — the module ships its own bespoke `source_file` /
`uses_package` / `augmented_project` triple with scalar-seam logic,
0 builtin types, 1 custom-resolver helper, 16 fixtures) to a
`pub(crate) static STRINGS: BuiltinModule`.

Note: although the overview census `srcglue` column reads 0 for this
package, `strings` has a custom source companion path at
`src/builtins/strings.rs:260`, `319`, and `448`. The `uses_package`
predicate at line 319 is the load-bearing scalar-seam trigger and must be
modeled as `BuiltinSource::Custom` under plan-72-A's descriptor vocabulary.

References: plan-72 overview, `src/builtins/strings.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/strings/`.

## Goal

- `strings::STRINGS` descriptor exists with resolver support for
  `implementation_name` (line 243) and with a `BuiltinSource` entry whose
  injection rule is `InjectionRule::Custom` and reproduces the current
  scalar-seam `uses_package` predicate.
- Legacy free functions in `strings.rs` become wrappers over `STRINGS`.
- Parity tests cover every function, every alias
  (`split` delimiter/separator, etc.), the implementation-name cases, and
  scalar-seam source injection with and without an explicit import.

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.
- Do not simplify the scalar-seam predicate — it triggers injection
  without an import and is behavior-defining.

## Current State

`src/builtins/strings.rs` has 10 helpers, no `package_source_glue!` macro,
its own `source_file` / `uses_package` / `augmented_project` triple at
lines 260–448, and one non-typed `implementation_name` at line 243.
Fixture load is 16 projects.

## Phases

### Phase V1 — descriptor and wrappers

- [ ] Add `pub(crate) static STRINGS: BuiltinModule` with every function,
      overload, parameter (canonical + aliases including `split`
      delimiter/separator), argument types, return type, implementation,
      and default.
- [ ] Model native members (`find`, `mid`, `replace`), source companion
      helpers (`toScalars`, `fromScalars`), and pure fixed-return functions
      with `Implementation` and `Lowering` values from plan-72-A.
- [ ] Model the bespoke source triple as `BuiltinSource` with
      `InjectionRule::Custom` carrying the scalar-seam predicate.
- [ ] Attach a resolver (or a static implementation-name table) for
      `implementation_name`.
- [ ] Rewrite the 10 metadata helpers as wrappers over `STRINGS`.
- [ ] Register `STRINGS` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: every `strings.*` name, every alias, every
      implementation-name case, and scalar-seam injection with and
      without an import.

Acceptance: `cargo test` passes; every `strings.*` fixture runs clean
under `scripts/test-accept.sh target/debug/mfb target/accept-actual`,
including the existing `tests/byte-identity/strings` cohort.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `strings` fixtures per the overview.

## Corrections

Filled during execution.
