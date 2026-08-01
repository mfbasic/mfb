# plan-72-C: source and type registry wiring

Last updated: 2026-07-31
Effort: medium (1h-2h)
Depends on: plan-72-B

This sub-plan moves builtin type lookup and source-companion injection for the
target modules to descriptor-backed registry APIs. It lands after B so the three
data modules already describe their types and source rules.

References: plan-72 overview, `src/resolver/mod.rs`, `src/syntaxcheck/mod.rs`,
`src/ir/lower.rs`, `src/builtins/mod.rs`, `src/builtins/strings.rs`.

## Goal

- Registry APIs answer builtin type membership and field lookup for the target
  packages.
- Resolver, syntaxcheck, and IR lower package-source augmentation use descriptor
  source rules for the target packages while preserving order.

## Non-goals

- Do not change non-target package source ordering.
- Do not infer package source dependencies dynamically yet; preserve the current
  explicit ordering.
- Do not migrate datetime/encoding function resolution behavior; D owns that.

## Current State

Source injection call chains exist in 3 files and include the five target
packages (`rg -n 'augmented_project\\(' src/resolver/mod.rs src/syntaxcheck/mod.rs
src/ir/lower.rs`). Current target package source glue sites are 4 macro uses plus
custom strings functions (`rg -n 'package_source_glue!|source_file\\(|uses_package\\(|augmented_project\\(' src/builtins/{strings,datetime,encoding,money,term}.rs`).

## Phases

### Phase C1 — builtin type registry

- [ ] Add `BuiltinRegistry::is_builtin_type`, `qualified_builtin_type`, and
      `builtin_type_fields` methods for descriptor modules.
- [ ] Route target package arms in `src/builtins/mod.rs:is_builtin_type` and
      `qualified_builtin_type` through the registry while leaving non-target
      arms as they are.
- [ ] Route `src/syntaxcheck/inference.rs`, `src/syntaxcheck/helpers.rs`,
      `src/ir/verify/mod.rs`, `src/ir/verify/compat.rs`, and
      `src/target/shared/code/validation.rs` term type-field lookups through the
      aggregate descriptor API.
- [ ] Tests: parity tests for `TermColor`, `TermSize`, `LineStyle`,
      `FillStyle`, `Rounding`, and datetime/encoding types once D descriptors
      exist; before D, mark datetime/encoding entries as wrappers to legacy
      descriptors if needed.

Acceptance: `cargo test` passes and term/money type fixtures remain unchanged in
acceptance.
Commit: —

### Phase C2 — source descriptor API

- [ ] Add registry methods for `uses_package`, `source_file`, and
      `augment_project_in_order`.
- [ ] Preserve the current injection order from `src/resolver/mod.rs`,
      `src/syntaxcheck/mod.rs`, and `src/ir/lower.rs`; only replace the target
      package calls with descriptor registry calls.
- [ ] Represent `strings` as `InjectionRule::Custom` with its existing
      scalar-seam predicate; represent `datetime`, `money`, `term`, and
      `encoding` as `WhenImported`.
- [ ] Tests: add source-augmentation parity tests for imported and not-imported
      target packages, plus strings scalar-seam references that trigger injection
      without an import.

Acceptance: `cargo test` passes; AST/IR goldens for target package fixtures are
unchanged under acceptance.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Inspect acceptance diffs for any changed AST file ordering; changed ordering is
  a regression unless a test is proven wrong under AGENTS.md.

## Corrections

Filled during execution.
