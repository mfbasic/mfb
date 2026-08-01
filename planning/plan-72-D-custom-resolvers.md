# plan-72-D: datetime and encoding resolvers

Last updated: 2026-07-31
Effort: large (3h-1d)
Depends on: plan-72-C

This sub-plan migrates the packages with real custom behavior: `datetime` and
`encoding`. It is scheduled after the data modules and source/type registry so
the descriptor core has already been exercised.

References: plan-72 overview, `src/builtins/datetime.rs`,
`src/builtins/encoding.rs`, `bugs/completed/bug-349-datetime-named-arg-misbinding.md`,
`planning/completed/plan-02-encoding.md`.

## Goal

- `datetime::DATETIME` and `encoding::ENCODING` descriptors exist.
- `DatetimeResolver` preserves overload-specific named argument binding,
  arity-dependent implementation names, return types, and default padding.
- `EncodingResolver` preserves overload return-type selection and monomorph
  overload target resolution.

## Non-goals

- Do not broaden accepted overloads.
- Do not change generated internal helper names.
- Do not delete wrappers yet; E owns deletion.

## Current State

`datetime` is 923 lines and `encoding` is 596 lines (`wc -l
src/builtins/{datetime,encoding}.rs`). Their fixture load is 41 projects
(`find tests/... -path '*/datetime/*/project.json'` → 12 and `encoding` → 29).
`datetime` has `call_param_name_overloads`, `implementation_name(name, argc)`,
and `default_argument_padding`; `encoding` has `resolve_overload_target` and
`is_overloaded`.

## Phases

### Phase D1 — datetime descriptor and resolver

- [ ] Convert datetime function/type metadata into `pub(crate) static DATETIME:
      BuiltinModule`.
- [ ] Implement `DatetimeResolver` for contextual returns, overload selection,
      parameter-name overloads, arity-dependent implementation names, and time
      default padding.
- [ ] Keep `is_datetime_call`, `call_param_names`, `call_param_name_overloads`,
      `call_return_type_name`, `resolve_call`, `expected_arguments`,
      `argument_types`, `arity`, `implementation_name`, and
      `default_argument_padding` as wrappers over descriptor/resolver behavior.
- [ ] Tests: parity for all datetime functions, the bug-349 named-argument cases,
      `instant`/`duration`/`fixedOffset`/`parse` implementation names by arity,
      and `time` default padding.

Acceptance: `cargo test` passes; datetime syntax/runtime/byte-identity fixtures
are unchanged under full validation.
Commit: —

### Phase D2 — encoding descriptor and resolver

- [ ] Convert encoding function metadata into `pub(crate) static ENCODING:
      BuiltinModule`.
- [ ] Implement `EncodingResolver` for `utf8Encode`/`utf8Decode` return-type
      overloads, parameter overloads, `implementation_name`, `is_overloaded`, and
      `resolve_overload_target`.
- [ ] Keep existing encoding free functions as wrappers over the descriptor and
      resolver.
- [ ] Tests: parity for all encoding functions, ambiguous `utf8Encode` invalid
      fixture behavior, bytes/ints overload targets, and implementation names.

Acceptance: `cargo test` passes; encoding syntax/runtime/byte-identity fixtures
are unchanged under full validation.
Commit: —

### Phase D3 — registry consumer routing for target packages

- [ ] Route `src/builtins/mod.rs:resolve_call_return_type`,
      `call_return_type_name`, `is_builtin_call`, `call_param_name_overloads`,
      and `call_param_names` to registry descriptors for all five target
      packages.
- [ ] Route `src/syntaxcheck/builtins.rs` target rows to descriptor adapters
      instead of direct module helper pointers.
- [ ] Route `src/ir/lower.rs:builtin_argument_types`,
      `normalize_builtin_call_arguments`, default padding, and target package
      `implementation_name` selection to descriptor APIs.
- [ ] Route `src/monomorph/lower.rs` encoding overload target resolution through
      the descriptor API.
- [ ] Tests: add registry-vs-legacy consumer parity tests that prove the old
      wrappers and new registry answers match until E deletes the wrappers.

Acceptance: `cargo test`, acceptance, and byte-identity/artifact gate all pass
with no target-package artifact drift.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate from the overview for all five target packages.

## Corrections

Filled during execution.
