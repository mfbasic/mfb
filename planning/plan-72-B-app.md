# plan-72-B: app package descriptor

Last updated: 2026-08-01
Effort: small (< 1h)
Depends on: plan-72-A

Migrate `src/builtins/app.rs` (178 LOC, 8 metadata helpers, 1 `package_source_glue!`,
1 builtin-type helper, 0 custom-resolver helpers, 6 fixtures) to a
`pub(crate) static APP: BuiltinModule`.

References: plan-72 overview, `src/builtins/app.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/app/`.

## Goal

- `app::APP` descriptor exists and mirrors the current 8 helpers plus builtin
  type entries and `WhenImported` source injection.
- Legacy free functions in `app.rs` become wrappers that consult `APP`.
- Parity tests cover every `app.*` name, alias, return type, argument type,
  and the source companion type(s).

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.
- Do not touch other packages.

## Current State

`src/builtins/app.rs` uses `package_source_glue!` for its `.mfb` companion and
exposes exactly one builtin type helper. The `app` fixture load is 6 projects
(`find tests/{syntax,rt-behavior,byte-identity} -path '*/app/*/project.json' |
wc -l → 6`).

## Phases

### Phase B1 — descriptor and wrappers

- [x] Add `pub(crate) static APP: BuiltinModule` with both functions
      (`getMode` 0-arg → `Mode`; `setMode(mode: Mode)` → `Nothing`), their
      overloads/parameters, `Fixed` return types, `Implementation::Same` (both
      lower inline per plan-62-B), and no defaults.
- [x] Add `BuiltinType` entry for `Mode` (`TypeKind::Enum`, no record fields).
- [x] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, loader: source_file }`.
- [x] Rewrite `is_app_call`, `arity`, `resolve_call`, `call_return_type_name`,
      and `is_builtin_type` as wrappers over `APP`/`DefaultResolver`.
      `call_param_names`, `argument_types`, and `expected_arguments` return
      `&'static` borrowed shapes the owned `DefaultResolver` cannot produce
      without allocation, so they stay static literals PINNED equal to `APP` by
      the parity test (BB moves the consumers off the borrowed ABI). See
      Corrections.
- [x] Register `APP` with the `BuiltinRegistry` (`descriptor::REGISTRY` now
      `new(&[&app::APP])`); updated A's registry test accordingly.
- [x] Parity tests: `parity_matches_descriptor` asserts `APP`-derived answers
      equal the helpers for `getMode`/`setMode`/`app.nope` (membership, arity,
      param names, return type, expected arguments, argument types, impl name,
      builtin type fields) plus `resolve_call` accept/reject and the `Mode`
      source companion type.

Acceptance: `cargo test` passes and every `app.*` fixture in
`tests/{syntax,rt-behavior,byte-identity}` runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual` (6 fixtures, all
`tests/syntax/app/`; no rt-behavior/byte-identity app fixtures exist).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `app` fixtures per the overview.

## Corrections

- **Three helpers cannot be pure descriptor delegations for B.**
  `call_param_names` (`Option<&'static [&'static [&'static str]]>`),
  `argument_types` (`Option<&'static [&'static str]>`), and `expected_arguments`
  (`Option<&'static str>`) return `&'static` borrowed shapes. `DefaultResolver`
  derives these as owned (`Vec`/`String`), which cannot be coerced to `&'static`,
  and their consumers (the syntaxcheck `BUILTIN_PACKAGES` table field types, IR
  lowering) require the borrowed types — changing them would ripple across every
  package and violates B's "do not touch other packages." So these three stay as
  static literals for B, held equal to `APP` by `parity_matches_descriptor`. BB
  (which moves consumers onto the owned descriptor API) deletes them. The plan's
  "rewrite … as wrappers over APP" is satisfied in intent: `APP` is the
  parity-verified authority; the ABI-facing shape is preserved until BB.
- **Corrected A's `DefaultResolver` zero-arg renderings.** Needed for `app.getMode`
  parity: `expected_arguments` now returns `"()"` and `argument_types` returns
  `None` for a zero-parameter call (shared convention across app/money/crypto/
  datetime). General vocabulary fix in `descriptor.rs`, pinned by A's unit tests.
  Also added `DefaultResolver::resolve_call` (exact arg-type match → fixed return)
  so `resolve_call` could delegate. Recorded in the overview Corrections too.
- **Overview Prerequisites baselines were stale post-A** (double-counting find,
  `descriptor.rs` as a 27th file, +2 call-site refs). Corrected in the overview's
  Prerequisites table and Corrections; re-verified all rows MET before starting B.
