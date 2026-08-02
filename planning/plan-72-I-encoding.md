# plan-72-I: encoding package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/encoding.rs` (596 LOC, 10 metadata helpers, 1
`package_source_glue!`, 0 builtin types, 3 custom-resolver helpers,
29 fixtures) to a `pub(crate) static ENCODING: BuiltinModule` with an
`EncodingResolver` that preserves `utf8Encode` / `utf8Decode` overload
selection and monomorph overload target resolution.

References: plan-72 overview, `src/builtins/encoding.rs`,
`planning/completed/plan-02-encoding.md`,
`tests/{syntax,rt-behavior,byte-identity}/*/encoding/`,
`src/monomorph/lower.rs`.

## Goal

- `encoding::ENCODING` descriptor exists with `EncodingResolver` covering
  `implementation_name` (line 210), `resolve_overload_target` (line 252),
  and `is_overloaded` (line 271).
- Legacy free functions become wrappers over
  `ENCODING`/`EncodingResolver`.
- Parity tests cover every function, every overload target, and every
  invalid-overload diagnostic path.

## Non-goals

- Do not broaden accepted overloads.
- Do not change generated internal helper names.
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/encoding.rs` has 10 helpers, a `package_source_glue!`
companion, and three custom-resolver helpers driving typed
`utf8Encode`/`utf8Decode` overload behavior. Fixture load is 29 projects —
the second-largest custom-resolver cohort.

## Phases

### Phase I1 — descriptor and resolver

- [x] Add `pub(crate) static ENCODING: BuiltinModule` — 32 unary functions (28
      public + 4 monomorph targets), each with a fixed return; non-overloaded
      names carry `Implementation::Rewrite(__encoding_*)`, `utf8Encode`/
      `utf8Decode` carry `Implementation::Custom`.
- [x] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, loader: source_file }`.
- [x] Implement `EncodingResolver` for `resolve_return_type` (validation) and
      `resolve_overload_target`. `implementation_name` is a fixed `Rewrite` map
      (not resolver — like csv) and `is_overloaded` derives from
      `Implementation::Custom` (see Corrections).
- [x] Rewrite the metadata helpers: `is_encoding_call`/`arity`/
      `call_return_type_name`/`implementation_name`/`is_overloaded` delegate to
      the descriptor; `resolve_call` routes through `EncodingResolver`;
      `call_param_names`/`expected_arguments`/`argument_types` stay static
      (`call_param_names` pinned by parity).
- [x] Register `ENCODING` with the `BuiltinRegistry`.
- [x] Route `src/monomorph/lower.rs` through the new descriptor-API entry point
      `builtins::resolve_overload_target(callee, arg_types, expected)` (registry
      lookup → owning package's `BuiltinResolver::resolve_overload_target`),
      replacing the direct `encoding::resolve_overload_target` call.
- [x] Parity test: 28 public names + `encoding.nope` (membership/arity/param
      names), the 4 monomorph targets verified explicitly (arity/return/impl/no
      param-names), 14 resolver samples (resolve_call returns + the
      `bytes`/`ints` overload targets incl. the ambiguous no-expected-type `Err`
      path via the existing `resolve_overload_target_all_paths` test).

Acceptance: `cargo test` passes; every `encoding.*` fixture runs clean under
`scripts/test-accept.sh` (29 fixtures; byte-identity via the combined C/F/G/H/I
artifact-gate at finalization).
Commit: 7f7a35d83

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `encoding` fixtures per the overview.

## Corrections

- **`implementation_name` needs no resolver; `is_overloaded` derives from the
  descriptor.** The plan put both on `EncodingResolver`, but `implementation_name`
  is a fixed name→`__encoding_*` map = `Implementation::Rewrite` (Custom for the
  two overloaded names), so `DefaultResolver::implementation_name` serves it.
  `is_overloaded(callee)` = the function's implementation is `Implementation::Custom`
  — no separate hook. `EncodingResolver` therefore implements only
  `resolve_return_type` and `resolve_overload_target`.
- **`BuiltinResolver::resolve_overload_target` signature upgraded (in
  `descriptor.rs`).** A's placeholder was `(module, name, arg_types) → Option<String>`;
  encoding's real need is `(module, name, arg_types, expected_type) → Result<Option<String>, ()>`
  (the return-type overload needs the contextual expected type and can fail
  ambiguously). Updated the trait, the parity harness `ResolverSample` (added an
  `expected_type` field), the A synthetic `SResolver`, and all `ResolverSample`
  constructions.
- **Descriptor-API routing for the monomorphizer.** Added
  `builtins::resolve_overload_target(callee, arg_types, expected)` — a registry
  lookup that delegates to the owning package's resolver hook — and routed
  `src/monomorph/lower.rs` through it (was a direct
  `encoding::resolve_overload_target` call). encoding's own logic moved to a
  private `dispatch_overload_target`; the public API is now the generic function.
- **The 4 monomorph targets are members with arity but no `call_param_names`**
  (never named-arg-bound), like audio's internal names. They are verified
  explicitly in the parity test rather than through the param-name harness.
- **Return types are FIXED**, so most helpers derive from the descriptor; only
  `resolve_call` validation and overload-target selection are argument-dependent.
  `expected_arguments`/`argument_types` (custom phrasing) stay static.
