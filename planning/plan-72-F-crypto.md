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

- [ ] Add `pub(crate) static CRYPTO: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return type,
      implementation, and default.
- [ ] Add `BuiltinType` entry for the crypto builtin type.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Implement `CryptoResolver` for typed `implementation_name` and
      `default_argument_padding`.
- [ ] Rewrite the 12 metadata helpers as wrappers over
      `CRYPTO`/`CryptoResolver`.
- [ ] Register `CRYPTO` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: every `crypto.*` name, every default-padding slot, every
      typed implementation-name case, and the builtin type.

Acceptance: `cargo test` passes; every `crypto.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`, including the
existing `tests/byte-identity/crypto` cohort.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `crypto` fixtures per the overview.

## Corrections

Filled during execution.
