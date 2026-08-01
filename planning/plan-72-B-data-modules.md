# plan-72-B: data-shaped modules

Last updated: 2026-07-31
Effort: large (3h-1d)
Depends on: plan-72-A

This sub-plan migrates the least custom target packages to descriptors:
`money`, `term`, and the data-shaped majority of `strings`. It keeps old free
functions as wrappers so downstream callers do not move yet.

References: plan-72 overview, `src/builtins/money.rs`, `src/builtins/term.rs`,
`src/builtins/strings.rs`, `bugs/completed/bug-340-builtins-cli-reorg.md`.

## Goal

- `money::MONEY`, `term::TERM`, and `strings::STRINGS` descriptors exist.
- Legacy helper functions in those modules derive from descriptors wherever the
  behavior is data-shaped.
- Parity tests prove descriptor and legacy behavior match for all functions in
  the migrated modules.

## Non-goals

- Do not move source injection to descriptors yet; C owns that.
- Do not delete wrapper functions.
- Do not migrate datetime or encoding.

## Current State

The target fixture load for these three modules is 55 projects (`for p in
strings money term; do find tests/syntax tests/rt-behavior tests/byte-identity
-path "*/$p/*/project.json" | wc -l; done → 16,6,33`). `money` has 8 metadata
helpers, `term` has 10, and `strings` has 10 in the 48-helper census from the
overview.

## Phases

### Phase B1 — money descriptor

- [ ] Convert `src/builtins/money.rs` function metadata into `pub(crate) static
      MONEY: BuiltinModule`.
- [ ] Keep `is_money_call`, `call_param_names`, `call_return_type_name`,
      `resolve_call`, `expected_arguments`, `argument_types`, and `arity` as
      wrappers over `MONEY`.
- [ ] Represent `Rounding` as a `BuiltinType` descriptor even though source
      injection still comes from the current package source glue.
- [ ] Tests: add module parity assertions for all `money.*` names and unknown
      names.

Acceptance: `cargo test` passes; `tests/syntax/money` and
`tests/rt-behavior/money` are covered by the full acceptance run before B is
merged.
Commit: —

### Phase B2 — term descriptor

- [ ] Convert `src/builtins/term.rs` call metadata into `pub(crate) static TERM:
      BuiltinModule`.
- [ ] Express `TermColor`, `TermSize`, `LineStyle`, and `FillStyle` as
      `BuiltinType` entries, with `TermColor` and `TermSize` record fields
      preserving `builtin_type_fields` output.
- [ ] Keep `is_term_call`, `is_builtin_type`, `builtin_type_fields`,
      `call_param_names`, `param_types`, `call_return_type_name`,
      `resolve_call`, `expected_arguments`, and `arity` as wrappers.
- [ ] Tests: add parity assertions for all term functions, type lookup, and
      record fields.

Acceptance: `cargo test` passes; `tests/syntax/term`, `tests/rt-behavior/term`,
and `tests/byte-identity/term` are unchanged under the plan validation commands.
Commit: —

### Phase B3 — strings descriptor

- [ ] Convert `src/builtins/strings.rs` public function metadata into
      `pub(crate) static STRINGS: BuiltinModule`.
- [ ] Model native members (`find`, `mid`, `replace`), source companion helpers
      (`toScalars`, `fromScalars`), and pure fixed-return functions with
      `Implementation` and `Lowering` values.
- [ ] Keep `is_strings_call`, `call_param_names`, `call_return_type_name`,
      `resolve_call`, `expected_arguments`, `arity`, and `implementation_name`
      as wrappers.
- [ ] Leave `source_file`, `uses_package`, and `augmented_project` untouched for
      C because `strings` has custom scalar-seam source-use logic.
- [ ] Tests: add parity assertions for all strings names, aliases such as
      `split` delimiter/separator, implementation names, and unknown names.

Acceptance: `cargo test` passes; strings syntax/runtime/byte-identity fixtures
are unchanged under the plan validation commands.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate from the overview for `strings`, `money`, and
  `term`.

## Corrections

Filled during execution.
