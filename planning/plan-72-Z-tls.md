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

- [x] Add `pub(crate) static TLS: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [x] Add `BuiltinType` entry for the tls builtin type. (Two: `TlsSocket`
      and `TlsListener`, both `Opaque`.)
- [x] ~~Implement `TlsResolver` for `default_argument_padding`.~~ — moot:
      the padding is data-derivable via `DefaultValue::Fill`, so
      `DefaultResolver::default_padding` reproduces it and NO resolver is
      needed (`resolver: None`, the `net` shape). See Corrections.
- [x] Rewrite the 10 metadata helpers as wrappers over `TLS`. (`is_tls_call`
      → `contains`, `arity`/`call_return_type_name` → `DefaultResolver` with an
      explicit `CLOSE_LISTENER` fallback, `is_builtin_type` → `TLS.types`;
      `resolve_call`/`expected_arguments`/`argument_types`/`default_argument_padding`
      stay hand-authored and are pinned by parity — see Corrections.)
- [x] Register `TLS` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests for every `tls.*` name, the builtin type, and every
      default-padding slot (`parity_matches_descriptor` checks
      `DefaultResolver::default_padding` for `provided` in `0..=max`).

Acceptance: `cargo test` passes; every `tls.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
(`cargo test --bin mfb builtins::tls` → 13 passed; full acceptance run at
finalization.)
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `tls` fixtures per the overview.

## Corrections

- **No `TlsResolver` — `default_argument_padding` is data-derivable, so `tls`
  is `resolver: None` (the `net` shape).** The plan's premise was that
  `default_argument_padding` needed a custom resolver hook. But the descriptor's
  `DefaultValue::Fill { type_name, expr }` (introduced in A, used by many prior
  letters) exactly reproduces the legacy padding: `connect`'s
  `timeoutMs=Fill(Integer,SENTINEL)`/`serverName=Fill(String,"")`,
  `listen`'s `backlog=Fill(Integer,"0")`, `accept`'s `timeoutMs=Fill(Integer,SENTINEL)`.
  `DefaultResolver::default_padding` skips the `provided` real args and emits the
  remaining `Fill`s, matching `default_argument_padding(name, provided)` for every
  `provided` in `0..=max` — verified by `parity_matches_descriptor`. A resolver
  would have duplicated this. Evidence: `cargo test --bin mfb builtins::tls` →
  `default_padding_branches` and `parity_matches_descriptor` both pass.
- **`close`'s union return stays in the hand-authored `resolve_call`.** Like
  `net::close`, `tls::close` accepts either handle type (`TlsSocket`/`TlsListener`)
  but always returns `Nothing`. The return is *fixed per name* (`ReturnType::Fixed`),
  so `call_return_type_name`/`arity` derive from the descriptor; only the
  argument-set acceptance is argument-dependent and it stays in the kept
  `resolve_call`. `expected_arguments` (`"TlsSocket or TlsListener"`) and
  `argument_types` (joined strings) stay hand-authored (`LegacySet` sets both to
  `None`), matching `net`.
- **The lowered-only `CLOSE_LISTENER` is not a descriptor function.** It is
  synthesized during IR lowering and is not user-callable, so `is_tls_call`
  (=`DefaultResolver::contains`) excludes it; `call_return_type_name` and `arity`
  fall back to it explicitly (the `net`/`audio` internal-name pattern), keeping
  codegen's post-lowering queries answered.
