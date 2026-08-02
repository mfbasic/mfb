# plan-72-Z: tls package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/tls.rs` (427 LOC, 10 metadata helpers, 0 source
glue, 1 builtin-type helper, 1 custom-resolver helper, 13 fixtures) to a
`pub(crate) static TLS: BuiltinModule` with a `TlsResolver` that preserves
`default_argument_padding` (line 179).

References: plan-72 overview, `src/builtins/tls.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/tls/`,
`memory/bug-tls-drop-rt-segfault-flake.md`,
`memory/bug-386-macos-tls-server-intermittent-hang-forever.md`.

## Goal

- `tls::TLS` descriptor exists with `TlsResolver` covering
  `default_argument_padding`.
- Legacy free functions in `tls.rs` become wrappers over `TLS`/`TlsResolver`.
- Parity tests cover every function, the builtin type, and every default
  padding slot.

## Non-goals

- Do not change TLS lifecycle semantics referenced by
  `[[bug-386-macos-tls-server-intermittent-hang-forever]]` or
  `[[bug-tls-drop-rt-segfault-flake]]`.
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/tls.rs` has 10 helpers, one builtin type (line 52), no
source companion, and one custom-resolver helper (`default_argument_padding`
at line 179). Fixture load is 13 projects.

## Phases

### Phase Z1 — descriptor and resolver

- [ ] Add `pub(crate) static TLS: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [ ] Add `BuiltinType` entry for the tls builtin type.
- [ ] Implement `TlsResolver` for `default_argument_padding`.
- [ ] Rewrite the 10 metadata helpers as wrappers over `TLS`/`TlsResolver`.
- [ ] Register `TLS` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests for every `tls.*` name, the builtin type, and every
      default-padding slot.

Acceptance: `cargo test` passes; every `tls.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `tls` fixtures per the overview.

## Corrections

Filled during execution.
