# plan-72-M: http package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/http.rs` (581 LOC, 9 metadata helpers, 1
`package_source_glue!`, 1 builtin-type helper, 2 custom-resolver helpers,
7 fixtures) to a `pub(crate) static HTTP: BuiltinModule` with an
`HttpResolver` that preserves `default_argument_padding` and typed
`implementation_name`.

References: plan-72 overview, `src/builtins/http.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/http/`.

## Goal

- `http::HTTP` descriptor exists with `HttpResolver` covering
  `default_argument_padding` (line 241) and `implementation_name(name,
  arg_types)` (line 277).
- Legacy free functions become wrappers over `HTTP`/`HttpResolver`.
- Parity tests cover every function, the builtin type, every default padding
  slot, and every arg-type driven implementation-name case.

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.
- Do not change runtime helper selection semantics.

## Current State

`src/builtins/http.rs` has 9 helpers, one builtin type (line 76), a
`package_source_glue!` companion, and two custom-resolver helpers. Fixture
load is 7 projects.

## Phases

### Phase M1 — descriptor and resolver

- [x] Add `pub(crate) static HTTP: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default. Done: 14 functions, fixed returns;
      optional trailing arguments are `DefaultValue::Fill` carrying the same
      `(type, expr)` pairs the legacy `default_argument_padding` injected.
- [x] Add `BuiltinType` entry for the http builtin type(s). Done: **four**
      record types (`Response`, `Request`, `RequestPart`, `Route`) — the
      "1 builtin-type" census counted the single `is_builtin_type` helper fn,
      not the type names it covers (see Corrections).
- [x] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, loader: source_file }`.
- [x] Implement `HttpResolver` for typed `implementation_name` (`handleRequest`
      selects `__http_handleRequest{,SSL}` by first-arg type) and
      `resolve_return_type` (type-union overload validation). Default padding is
      NOT resolver-owned — it is data-derivable from the `Fill` parameters (see
      Corrections).
- [x] Rewrite the 9 metadata helpers as wrappers over `HTTP`/`HttpResolver`.
      `is_http_call`→`contains`, `arity`→`arity`, `call_return_type_name`
      →`return_type_name` delegate to `DefaultResolver`; `resolve_call` routes
      through `HttpResolver::resolve_return_type`; `implementation_name` uses the
      shared `handle_request_target` for `handleRequest` and
      `DefaultResolver::implementation_name` (the `Rewrite` symbol) otherwise;
      `is_builtin_type` queries `HTTP.types`; `call_param_names`,
      `expected_arguments`, and `default_argument_padding` stay hand-authored
      statics pinned by parity.
- [x] Register `HTTP` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests: every `http.*` name, every default-padding slot, both
      typed `handleRequest` implementation cases, and the four builtin types
      (`parity_matches_descriptor`).

Acceptance: `cargo test` passes; every `http.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`, including
the existing `tests/byte-identity/http` cohort.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `http` fixtures per the overview.

## Corrections

- **`default_argument_padding` is data-derivable, not resolver-owned.** Phase M1
  called for `HttpResolver` to own `default_argument_padding`, but every http
  default is a plain trailing `(type, expr)` fill (`headers={}`, `method=GET`,
  `host=0.0.0.0`, `backlog=128`, `contentType=""`). Modelling those as
  `DefaultValue::Fill` parameters makes `DefaultResolver::default_padding`
  reproduce the legacy static byte-for-byte, so the resolver does not override
  `default_padding` (identical to how plan-72-F handled crypto's AEAD `aad`). The
  legacy `default_argument_padding` static is kept (borrowed `&'static` return) and
  pinned to the `Fill` parameters by the parity test. Evidence: parity assertion
  `DefaultResolver::default_padding(&HTTP, name, provided) == default_argument_padding(name, provided)`
  over READ/WRITE/SERVER/SERVER_SSL/RESPOND_FILE.
- **http contributes four builtin types, not one.** The census `btypes 1` counts
  the single `is_builtin_type` helper fn; that fn covers four record type names
  (`Response`, `Request`, `RequestPart`, `Route`), all modelled as `BuiltinType`
  entries. Evidence: original `http.rs` `is_builtin_type` match arm and the
  `server_types_and_consumes` test. `resolve_call`'s type-union overloads
  (`handleRequest` accepts `Listener` OR `TlsListener`) cannot be expressed as a
  single `ParameterType::Named`, so — like crypto/collections — `resolve_call`
  routes through `HttpResolver::resolve_return_type` (bespoke `dispatch_resolve`)
  rather than `DefaultResolver::resolve_call`.
