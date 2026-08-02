# plan-72-F: crypto package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/crypto.rs` (814 LOC, 12 metadata helpers, 1
`package_source_glue!`, 1 builtin-type helper, 2 custom-resolver helpers,
5 fixtures) to a `pub(crate) static CRYPTO: BuiltinModule` with a
`CryptoResolver` that preserves `default_argument_padding` and typed
`implementation_name`.

References: plan-72 overview, `src/builtins/crypto.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/crypto/`.

## Goal

- `crypto::CRYPTO` descriptor exists with `CryptoResolver` covering
  `default_argument_padding` (line 336) and `implementation_name(name,
  arg_types)` (line 358).
- Legacy free functions become wrappers over `CRYPTO`/`CryptoResolver`.
- Parity tests cover every function, the builtin type, every default padding
  slot, and every arg-type driven implementation-name case.

## Non-goals

- Do not remove the wrapper functions; `BB` owns deletion.
- Do not change crypto runtime helper selection semantics.

## Current State

`src/builtins/crypto.rs` exposes 12 descriptor-owned helpers, one builtin
type via `is_builtin_type` (line 111), and injects a companion via
`package_source_glue!`. Fixture load is 5 projects.

## Phases

### Phase F1 — descriptor and resolver

- [x] Add `pub(crate) static CRYPTO: BuiltinModule` — 33 functions, each with a
      single overload of fixed return type (the overloading is on ARGUMENT types,
      not the return); AEAD `aad` modelled as an optional param.
- [x] Add `BuiltinType` entries for `Sealed` and `KeyPair` (`TypeKind::Opaque`).
- [x] Model the `package_source_glue!` companion (5-file concat) as
      `BuiltinSource { rule: InjectionRule::WhenImported, loader: source_file }`.
- [x] Implement `CryptoResolver` for typed `implementation_name` (`_bytes`/
      `_text`) and `resolve_return_type` (bytes/String + variadic AEAD arg
      validation), delegating to the retained `dispatch_*` helpers.
      `default_argument_padding` does NOT need the resolver — it derives from the
      optional `aad` param via `DefaultResolver::default_padding` (see Corrections).
- [x] Rewrite the metadata helpers: `is_crypto_call`/`arity`/
      `call_return_type_name`/`is_builtin_type` delegate to the descriptor;
      `resolve_call`/`implementation_name` route through `CryptoResolver`;
      `call_param_names` (borrowed ABI, pinned), `expected_arguments`/
      `argument_types`/`default_argument_padding` (custom/borrowed) stay static.
      `is_native_crypto_call`/`is_crypto_internal_call` are crypto ROUTING
      predicates, not descriptor metadata — kept as-is (Corrections).
- [x] Register `CRYPTO` with the `BuiltinRegistry`.
- [x] Parity test: every `crypto.*` name + `crypto.bogus` (membership/arity/
      param names, both types); 7 resolver samples (bytes/text hash + hmac impl,
      AEAD/keygen/uuid/ctEqual return+impl); explicit AEAD default-padding parity
      across all provided counts; native → None impl.

Acceptance: `cargo test` passes; every `crypto.*` fixture runs clean under
`scripts/test-accept.sh` (5 fixtures; byte-identity via the combined C/F/G/H/I
artifact-gate at finalization).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `crypto` fixtures per the overview.

## Corrections

- **`default_argument_padding` needs no resolver.** The plan put it on
  `CryptoResolver`, but AEAD `aad` padding is exactly a trailing-optional-parameter
  fill, so modelling `aad` as `DefaultValue::Fill { "List OF Byte", "" }` lets
  `DefaultResolver::default_padding` derive the identical slots. `CryptoResolver`
  therefore implements only `resolve_return_type` and `implementation_name`. The
  public `default_argument_padding` stays static (borrowed `&'static` ABI) and is
  pinned equal to the descriptor by the parity test.
- **Return types are FIXED, not resolver-owned.** crypto's overloading is on
  argument types; each function's return is fixed, so `call_return_type_name` and
  `arity` derive from the descriptor. Only argument-VALIDATION (`resolve_call`) and
  typed body selection (`implementation_name`) are argument-dependent → resolver.
- **`is_native_crypto_call` / `is_crypto_internal_call` are routing predicates,
  not descriptor metadata** (they classify native-helper vs source vs
  internal-only lowering). Kept as-is, like collections' `is_native_member`. So 10
  of the census's "12 helpers" migrate; these 2 stay.
- **`expected_arguments` / `argument_types` are not descriptor-derivable** (custom
  `"List OF Byte or String"` / joined-string phrasing the per-position types can't
  render). Kept static, `LegacySet` fields `None`.
