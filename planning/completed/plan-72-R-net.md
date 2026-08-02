# plan-72-R: net package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/net.rs` (725 LOC, 11 metadata helpers, 1
`package_source_glue!`, 2 builtin-type helpers, 2 custom-resolver helpers,
46 fixtures) to a `pub(crate) static NET: BuiltinModule` with a
`NetResolver` that preserves parameter-name overloads and typed
implementation-name selection.

References: plan-72 overview, `src/builtins/net.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/net/`,
`memory/thread-resource-plane-split.md`.

## Goal

- `net::NET` descriptor exists with `NetResolver` covering
  `call_param_name_overloads` (line 135) and `implementation_name(name)`
  (line 324).
- Legacy free functions become wrappers over `NET`/`NetResolver`.
- Parity tests cover every function, both builtin types, every
  parameter-name overload, and every implementation-name case.

## Non-goals

- Do not change the thread-resource plane split
  (`[[thread-resource-plane-split]]`).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/net.rs` has 11 helpers, two builtin types (lines 74 and 87),
a `package_source_glue!` companion, and two custom-resolver helpers.
Fixture load is 46 projects — the largest custom-resolver cohort in the
plan.

## Phases

### Phase R1 — descriptor and resolver

- [x] Add `pub(crate) static NET: BuiltinModule` with every function,
      overload, parameter (canonical + aliases per overload), argument
      types, return type, implementation, and default. `connectTcp` is
      modelled as four `BuiltinOverload`s; the optional trailing arguments
      (`lookup.port`, `listenTcp.backlog`, `accept/poll.timeoutMs`) are
      `DefaultValue::Optional`.
- [x] Add `BuiltinType` entries for all seven net builtin types with record
      fields preserved (`Address`, `Datagram`, `DatagramText` records; the
      rest opaque; `Url`'s fields live in the source companion → reported as
      absent here, matching the legacy `builtin_type_fields`).
- [x] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [x] ~~Implement `NetResolver` for `call_param_name_overloads` and
      `implementation_name`.~~ — moot: neither needs a resolver.
      `call_param_name_overloads` derives from
      `DefaultResolver::param_name_overloads` (connectTcp's four overloads);
      `implementation_name` is a fixed per-name rewrite
      (`Implementation::Rewrite(__net_*)` for toUrl/percentDecode/parseQuery,
      `Same` elsewhere). net is data-only. See Corrections.
- [x] Rewrite the 11 metadata helpers as wrappers over `NET`
      (`is_net_call`/`arity`/`call_return_type_name`/`implementation_name`
      delegate; `call_param_names`/`call_param_name_overloads`/
      `builtin_type_fields` are borrowed statics pinned by parity;
      `resolve_call`/`expected_arguments`/`argument_types` stay hand-authored —
      type-set acceptance and `"or"`-phrased/joined strings).
- [x] Register `NET` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests: every `net.*` name, all builtin types, every
      parameter-name overload, and every implementation-name case.

Acceptance: `cargo test` passes; every `net.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`, including
the existing `tests/byte-identity/net` cohort.
Commit: 2b1626c51

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `net` fixtures per the overview.

## Corrections

- **net needs no resolver (the plan's `NetResolver` is moot).** The plan
  specified a `NetResolver` for `call_param_name_overloads` and
  `implementation_name`. In fact the descriptor models both natively:
  `connectTcp`'s four structurally-different overloads become four
  `BuiltinOverload`s, so `DefaultResolver::param_name_overloads` reproduces the
  legacy per-overload name table (and `param_names` correctly yields `None`);
  and net's `implementation_name` is a fixed per-name rewrite
  (`Implementation::Rewrite(__net_toUrl / __net_percentDecode / __net_parseQuery)`,
  `Same` for the native calls), which `DefaultResolver::implementation_name`
  derives. So `NET` is `resolver: None`, and the `resolve_overload_target`
  production path (the only production consumer of the registry) is a no-op for
  it — same as before registration. Evidence:
  `src/builtins/net.rs:call_param_name_overloads` was a single-name match on
  `connectTcp` and `implementation_name` a fixed 3-arm `match`, neither
  argument-dependent.
- **net's return type is fixed per name.** Although several calls are overloaded
  on *argument* types (`connectTcp`, `close`, `localAddress`, the timeout
  setters), every overload of a given call returns the same type, so
  `call_return_type_name` delegates to `DefaultResolver::return_type_name`. The
  argument-type-set *acceptance* (which arguments resolve at all) is what stays
  hand-authored in `resolve_call`.
