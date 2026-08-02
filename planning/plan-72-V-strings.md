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

- [x] Add `pub(crate) static STRINGS: BuiltinModule` with every function,
      overload, parameter (canonical + aliases including `split`
      delimiter/separator, `join` parts/values, `replace` old/needle &
      new/replacement), argument types, return type, implementation, and
      default. Done: all 38 functions, each one fixed-return overload;
      optional trailing args (`padChar`, `start`) are `DefaultValue::Optional`.
- [x] Model native members (`find`, `mid`, `replace`), source companion
      helpers (`toScalars`, `fromScalars`), and pure fixed-return functions.
      Done: seam members are `Implementation::Rewrite(__strings_*)`, all
      others `Implementation::Same` (native / bare-name lowering unchanged);
      `Lowering::Helper` throughout.
- [x] Model the bespoke source triple as `BuiltinSource` with
      `InjectionRule::WhenUsed` (the enum's name for "Custom" predicate)
      carrying the scalar-seam predicate. Done: `uses_source` on the resolver
      delegates to the load-bearing `uses_package` walk.
- [x] Attach a resolver for the scalar-seam source predicate. Done:
      `StringsResolver` implements only `uses_source`; `implementation_name`
      needed NO resolver — it is a fixed per-name `Rewrite` derivable by
      `DefaultResolver` (see Corrections).
- [x] Rewrite the metadata helpers as wrappers over `STRINGS`:
      `is_strings_call`, `call_return_type_name`, `resolve_call`, `arity`,
      `implementation_name` delegate to `DefaultResolver`. `call_param_names`
      keeps its `&'static` table (pinned by parity); `expected_arguments`
      stays bespoke (`[, T]`).
- [x] Register `STRINGS` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests (`parity_matches_descriptor`): every `strings.*` name +
      alias, every implementation-name case, return types, resolve_call
      (incl. optional-arg overloads), no-padding invariant, and scalar-seam
      `WhenUsed` injection with and without a seam reference.

Acceptance: `cargo test` passes (`cargo test --bin mfb builtins::strings → 20
passed`; full `builtins::` suite → 403 passed); `strings.*` fixtures (incl.
`tests/byte-identity/strings`) verified byte-identical in the consolidated T–X
acceptance at finalization (metadata-only wrappers proven equal by parity; the
descriptor `REGISTRY` is never read in production dispatch).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `strings` fixtures per the overview.

## Corrections

- **`implementation_name` needs no resolver (census `custom 1` is data-shaped).**
  The Goal called for "resolver support for `implementation_name`". It is a fixed
  per-name map (the 7 seam members → `__strings_*`, everyone else → native), so it
  is modeled as `Implementation::Rewrite`/`Same` and derived by
  `DefaultResolver::implementation_name` — the same finding as csv-G/regex-T. The
  resolver `STRINGS` DOES carry exists solely for the scalar-seam SOURCE predicate
  (`uses_source`), which the descriptor's `InjectionRule::WhenUsed` requires.
- **`InjectionRule::Custom` → `WhenUsed`.** The plan text named the rule
  `InjectionRule::Custom`; the plan-72-A enum spells it `WhenUsed` (inject only
  when a package-specific predicate holds). Modeled as `WhenUsed` with the
  predicate on the resolver's `uses_source`.
- **Optional trailing args are `Optional`, not `Fill`.** `padLeft`/`padRight`'s
  `padChar` and `find`'s `start` widen arity to (2,3) but are never default-padded
  (strings has no `default_argument_padding`; the bodies select by arg count). The
  parity test asserts `default_padding` is empty for every call at every provided
  count.
- **`expected_arguments` stays hand-authored** (bespoke `[, T]` bracket phrasing;
  the `collections`/`regex` precedent). `LegacySet.expected_arguments = None`.
- **`resolve_call` no longer uses the shared `exact` helper.** It now delegates to
  `DefaultResolver::resolve_call`; the module-level `use super::exact;` was removed
  and the `exact_helper` regression test imports `crate::builtins::exact` directly.
- **Prerequisites Row 4 measured 468** (plan text says 451, a letter-B snapshot) —
  expected per-letter growth; recorded here rather than editing the shared plan-72
  overview a concurrent session is updating.
