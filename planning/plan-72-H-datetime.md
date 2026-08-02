# plan-72-H: datetime package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/datetime.rs` (923 LOC, 11 metadata helpers, 1
`package_source_glue!`, 1 builtin-type helper, 3 custom-resolver helpers,
12 fixtures) to a `pub(crate) static DATETIME: BuiltinModule` with a
`DatetimeResolver` that preserves bug-349 named-argument overload binding,
arity-dependent implementation names, and time default padding.

References: plan-72 overview, `src/builtins/datetime.rs`,
`bugs/completed/bug-349-datetime-named-arg-misbinding.md`,
`bugs/completed/bug-173-builtins-syntaxcheck-typecheck-nits.md`,
`src/docs/spec/stdlib/02_datetime.md`,
`tests/{syntax,rt-behavior,byte-identity}/*/datetime/`.

## Goal

- `datetime::DATETIME` descriptor exists with `DatetimeResolver` covering
  `call_param_name_overloads` (line 190), `implementation_name(name, argc)`
  (line 363), and `default_argument_padding` (line 379).
- Legacy free functions become wrappers over `DATETIME`/`DatetimeResolver`.
- Parity tests cover every function, every named-argument overload variant,
  every arity-dependent implementation name, and the `time` default padding.

## Non-goals

- Do not broaden accepted overloads.
- Do not change generated internal helper names.
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/datetime.rs` is the highest-risk resolver package in this plan
because named-argument binding is correctness-sensitive per bug-349. It has 11
helpers, 1 builtin type (line 71), a `package_source_glue!` companion, and
three custom-resolver helpers. Fixture load is 12 projects.

## Phases

### Phase H1 — descriptor and resolver

- [x] Add `pub(crate) static DATETIME: BuiltinModule` — 44 functions; `instant`/
      `duration` carry 5 per-overload tables and `fixedOffset` 2 (the bug-349
      cases); `time` has padded optional `second`/`nanos`, `parse` an unpadded
      optional `zone`; each function's return is fixed.
- [x] Add `BuiltinType` entries for all 9 datetime types (records + enums; no
      descriptor-modelled fields).
- [x] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, loader: source_file }`.
- [x] Implement `DatetimeResolver` for `resolve_return_type` (validation) and
      arity-keyed `implementation_name`. Parameter-name overloads are descriptor
      DATA (`param_name_overloads`), and `default_argument_padding` derives from
      `time`'s optional params — neither needs the resolver (see Corrections).
- [x] Rewrite the metadata helpers: `is_datetime_call`/`arity`/
      `call_return_type_name`/`is_builtin_type` delegate to the descriptor;
      `resolve_call` + `implementation_name` route through `DatetimeResolver`
      (the latter forwards `argc` as a length-`argc` placeholder); `call_param_names`/
      `call_param_name_overloads`/`default_argument_padding` static, pinned;
      `expected_arguments`/`argument_types` static (custom phrasing).
- [x] Register `DATETIME` with the `BuiltinRegistry`.
- [x] Parity test: all 44 names + `datetime.nope` (membership/arity/param names,
      the per-overload tables for instant/duration/fixedOffset, 9 types), 13
      resolver samples covering the arity-keyed impl names (`__datetime_instant3`,
      `__datetime_parse2/3`, `fixedOffset1/2`) and typed returns, `time` padding
      derivation, and parse's unpadded optional zone.

Acceptance: `cargo test` passes and every `datetime.*` fixture runs clean
under `scripts/test-accept.sh` (12 fixtures; byte-identity via the combined
C/F/G/H/I artifact-gate at finalization).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `datetime` fixtures per the overview.

## Corrections

- **`DatetimeResolver` covers only `resolve_return_type` + `implementation_name`.**
  The plan also named parameter-name overloads and `default_argument_padding` as
  resolver work, but both derive from the descriptor: per-overload names via
  `DefaultResolver::param_name_overloads` (bug-349's `instant`/`duration`/
  `fixedOffset` modelled as multi-overload functions), and `time` padding via
  `DefaultResolver::default_padding` over `time`'s optional `second`/`nanos`.
- **New `DefaultValue::Optional` variant.** `time`'s trailing params are padded
  (`Fill` → `0`), but `parse`'s trailing `zone` widens arity WITHOUT padding (the
  implementation picks `__datetime_parse{argc}` by count). `Fill` conflated the
  two, so `Optional` was added: it contributes to the arity range like `Fill` but
  `default_padding` skips it. `parse.zone` uses it.
- **`implementation_name(name, argc)` routes through the resolver via a
  placeholder.** datetime's selection depends only on the argument COUNT, but the
  resolver hook takes argument TYPES, so the wrapper passes `vec![String::new();
  argc]` and the resolver reads `.len()`. Behaviour is identical.
- **Return types are FIXED**, so `call_return_type_name`/`arity` derive from the
  descriptor; only `resolve_call` validation and arity-keyed body selection are
  argument-dependent. `expected_arguments`/`argument_types` (custom phrasing) and
  the borrowed-ABI helpers stay static, pinned by parity.
