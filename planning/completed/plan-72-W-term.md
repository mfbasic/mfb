# plan-72-W: term package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/term.rs` (477 LOC, 9 metadata helpers, 1
`package_source_glue!`, 3 builtin-type helpers, 0 custom-resolver helpers,
33 fixtures) to a `pub(crate) static TERM: BuiltinModule`.

References: plan-72 overview, `src/builtins/term.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/term/`,
`memory/plan-62-presentation-mode-system.md`.

## Goal

- `term::TERM` descriptor exists and mirrors every current metadata helper
  plus all four builtin types (`TermColor`, `TermSize`, `LineStyle`,
  `FillStyle`), with `TermColor` and `TermSize` record fields preserved.
- Legacy free functions in `term.rs` become wrappers over `TERM`.
- Parity tests cover every function, every type, and every record field.

## Non-goals

- Do not change presentation-mode semantics
  (`[[plan-62-presentation-mode-system]]`).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/term.rs` has 9 descriptor-owned helpers and three
builtin-type helpers (`is_builtin_type`, `builtin_type_fields`,
`param_types`) covering both the hard-coded types (`TermColor`,
`TermSize`) and the source companion types (`LineStyle`, `FillStyle`). It
injects a companion via `package_source_glue!`. Fixture load is 33
projects.

## Phases

### Phase W1 — descriptor and wrappers

- [x] Add `pub(crate) static TERM: BuiltinModule` with every function,
      overload, parameter, argument types, return type, implementation, and
      default. Done: 23 functions, each one fixed-return overload;
      `Implementation::Same` (no rewrite — `term::` lowers by name to the
      native backend).
- [x] Add `BuiltinType` entries for `TermColor`, `TermSize`, `LineStyle`,
      and `FillStyle`. Done: `TermColor`/`TermSize` are `TypeKind::Record`
      with their fields preserved; `LineStyle`/`FillStyle` are
      `TypeKind::Enum` with no native fields (declared in the source
      companion).
- [x] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, loader: source_file }`.
- [x] Rewrite the derivable helpers as wrappers over `TERM`: `is_term_call`
      (`contains`), `is_builtin_type` / `builtin_type_fields` (`TERM.types`),
      `call_return_type_name` (`return_type_name`), `resolve_call`
      (name-only, via `return_type_name` — NOT the exact-match
      `resolve_call`), and `arity`. `call_param_names`, `param_types`, and
      `expected_arguments` keep their hand-authored tables (see Corrections)
      pinned by parity.
- [x] Register `TERM` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests (`parity_matches_descriptor`): every `term.*` name, all
      four builtin types + record fields, per-position argument types vs
      `param_types`, and the `WhenImported` source companion.

Acceptance: `cargo test` passes (`cargo test --bin mfb builtins::term → 10
passed`); `term.*` fixtures (incl. `tests/byte-identity/term`) verified
byte-identical in the consolidated T–X acceptance at finalization (metadata-only
wrappers proven equal by parity; the descriptor `REGISTRY` is never read in
production dispatch).
Commit: 89b4be3b2

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `term` fixtures per the overview.

## Corrections

- **Three helpers stay hand-authored, not "wrappers over `TERM`".** The task list
  said to rewrite `call_param_names`, `param_types`, and `expected_arguments` as
  wrappers. They can't fully delegate:
  - `call_param_names` returns a `&'static [&'static [&'static str]]` borrowed
    shape the owned `DefaultResolver::param_names` (returns `Vec`) cannot produce;
    kept as a static table PINNED equal to `TERM` by the parity test (the csv/regex
    precedent).
  - `param_types` returns `Some(&[])` for a zero-arg call, but the descriptor's
    `argument_types` returns `None` for zero params (the shared convention). So
    `param_types` stays hand-authored; the parity test asserts descriptor
    `argument_types == param_types` for non-empty calls and `None` vs `Some(&[])`
    for the zero-arg divergence.
  - `expected_arguments` renders a zero-arg call as `"no arguments"`, but the
    descriptor renders `"()"`. Bespoke phrasing → kept hand-authored (the
    `collections`/`regex` precedent); `LegacySet.expected_arguments = None`.
- **`resolve_call` delegates to `return_type_name`, not `resolve_call`.** A
  `term::` call's return type is a function of the name alone (the legacy
  `resolve_call` ignores `arg_types`), so it delegates to
  `DefaultResolver::return_type_name`; `DefaultResolver::resolve_call` does exact
  argument-type matching and would wrongly reject a name-only lookup.
- **Prerequisites Row 4 measured 468** (plan text says 451, a letter-B snapshot);
  expected per-letter growth as descriptor plumbing is added — recorded here rather
  than editing the shared plan-72 overview a concurrent session is updating.
