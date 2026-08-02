# plan-72-T: regex package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/regex.rs` (298 LOC, 11 metadata helpers, 0
`package_source_glue!` — the module ships its own bespoke `source_file` /
`uses_package` / `augmented_project` triple, 0 builtin types, 2
custom-resolver helpers, 6 fixtures) to a `pub(crate) static REGEX:
BuiltinModule`.

Note: although the overview census `srcglue` column reads 0 for this
package, `regex` has a custom source companion path
(`src/builtins/regex.rs:104`, `123`, `131`). Model it under the descriptor
`BuiltinSource` shape from plan-72-A rather than the `WhenImported` macro
rule used by `package_source_glue!`.

References: plan-72 overview, `src/builtins/regex.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/regex/`.

## Goal

- `regex::REGEX` descriptor exists with resolver support for
  `implementation_name` (line 79) and `default_argument_padding` (line 92),
  and with a `BuiltinSource` entry that reproduces the current
  `source_file` + `uses_package` + `augmented_project` behavior.
- Legacy free functions in `regex.rs` become wrappers over `REGEX`.
- Parity tests cover every function, both custom-resolver hooks, and the
  source companion.

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/regex.rs` has 11 helpers, no `package_source_glue!` macro,
its own `source_file` / `uses_package` / `augmented_project` triple at
lines 104–131, and two custom-resolver helpers. Fixture load is 6
projects.

## Phases

### Phase T1 — descriptor, resolver, and source

- [x] Add `pub(crate) static REGEX: BuiltinModule` with every function,
      overload, parameter (single spelling, no aliases), return type,
      implementation, and default. Done: 4 functions, each one fixed-return
      overload; `find`/`findAll` carry the trailing `start: Fill("Integer","0")`.
- [x] Model the bespoke source triple as `BuiltinSource`. Done:
      `InjectionRule::WhenImported` — `uses_package` is the standard "imports
      `regex`" check, so `WhenImported` reproduces it exactly (no `Custom`
      predicate needed). Loader is the bespoke `source_file` (engine +
      generated Unicode table combined into one file).
- [x] ~~Implement a resolver for `implementation_name` and
      `default_argument_padding`.~~ — moot: neither is argument-dependent.
      `implementation_name` is a fixed per-name `Implementation::Rewrite`
      and `default_argument_padding` is a trailing `DefaultValue::Fill`, so
      both derive from `DefaultResolver` with NO resolver (same finding as
      csv-G's "1 custom-resolver helper"). Evidence: `regex.rs`
      `implementation_name` matches on name only; parity asserts padding
      equal over every provided count.
- [x] Rewrite the metadata helpers as wrappers over `REGEX`: `is_regex_call`,
      `call_return_type_name`, `resolve_call`, `arity`, `implementation_name`
      delegate to `DefaultResolver`. `call_param_names` and
      `default_argument_padding` keep their `&'static` borrowed-shape tables
      (PINNED by parity); `expected_arguments` stays bespoke (`[, Integer]`).
- [x] Register `REGEX` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests (`parity_matches_descriptor`): every `regex.*` name,
      implementation-name rewrites, default padding over every provided count,
      resolve_call incl. the optional trailing `start`, and the `WhenImported`
      source companion.

Acceptance: `cargo test` passes (`cargo test --bin mfb builtins::regex → 13
passed`); `regex.*` fixtures verified byte-identical in the consolidated T–X
acceptance at finalization (metadata-only wrappers proven equal by parity; the
descriptor `REGISTRY` is never read in production dispatch).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `regex` fixtures per the overview.

## Corrections

- **No resolver needed (census `custom 2` is two data-shaped helpers).** The
  sub-plan Goal called for "resolver support for `implementation_name` and
  `default_argument_padding`". Neither is argument-dependent:
  `regex.rs:implementation_name` matches on the call name only (a fixed
  `Implementation::Rewrite(__regex_*)`), and `default_argument_padding` is a plain
  trailing fill on `find`/`findAll`'s `start` (a `DefaultValue::Fill("Integer",
  "0")`). Both derive from `DefaultResolver`, so `REGEX.resolver = None`. This is
  the same finding as csv-G (whose "1 custom-resolver helper" was a fixed
  `Rewrite`). Evidence: the parity test asserts descriptor default padding equal to
  the legacy `default_argument_padding` over every provided count (0..=3).
- **`expected_arguments` stays hand-authored.** `find`/`findAll` render the
  optional `start` as `String, String[, Integer]`; the descriptor's per-position
  type list produces `String, String, Integer`, so `expected_arguments` cannot be
  descriptor-derived and remains a free function (the `collections` precedent,
  documented in the plan-72-A parity harness). The parity `LegacySet` sets
  `expected_arguments: None` for this reason.
- **Prerequisites Row 4 measured 468** in this worktree (plan text says 451, a
  letter-B snapshot). The overview states this count grows per migrated letter as
  descriptor plumbing is added; C–I have since landed on main (the fork base), so
  468 is expected growth, not drift. Recorded here rather than editing the shared
  plan-72 overview, which a concurrent session is actively updating.
