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

- [ ] Add `pub(crate) static REGEX: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Model the bespoke source triple as `BuiltinSource` with an injection
      rule (`WhenImported` if the `uses_package` predicate can be modeled
      as such, otherwise `Custom` with the existing predicate closure).
- [ ] Implement a resolver for `implementation_name` and
      `default_argument_padding`.
- [ ] Rewrite the 11 metadata helpers as wrappers over `REGEX`.
- [ ] Register `REGEX` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: every `regex.*` name, every implementation-name case,
      every default-padding slot, and the source companion injection.

Acceptance: `cargo test` passes; every `regex.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `regex` fixtures per the overview.

## Corrections

Filled during execution.
