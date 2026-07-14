# plan-06-A: TLS server — surface, specs, Linux backend

Last updated: 2026-06-30
Effort: medium

Part **A** of plan-06 (TLS Server). Lands the front-end surface, the runtime ABI specs, and the
Linux OpenSSL backend — enough for a working server on Linux. Shared design (goal, current state,
record layout, close semantics, validation plan) lives in the overview:
[plan-06-tls-server.md](plan-06-tls-server.md).

- **Depends on:** nothing — land first.
- **Blocks:** plan-06-B (macOS backend + docs + tests reuse this surface).
- **Spec/design:** overview §4 (front-end), §5 (ABI specs & layout), §6 (Linux OpenSSL backend).

## Phases

### Phase A1 — Front-end surface + resource registration (§4)

- [x] Add `LISTEN`/`ACCEPT` calls, the `TlsListener` type, overloads, arity, and defaulting in `src/builtins/tls.rs` and `resource.rs`.
- [x] Full `_invalid` overload coverage for bad arities/types.

Acceptance: a program using `tls::listen`/`tls::accept` type-checks and reports the right diagnostics for bad arities/types; `_invalid` coverage compiles clean. DONE — `tls::close` overloads over both `TlsSocket` and `TlsListener` (a `TlsListener` operand rewrites to the internal `tls.closeListener` body in IR lowering, §4.1); `func_tls_{listen,accept}_{valid,invalid}` cover every overload plus the resource-in-record / resource-in-`List` rejections.

### Phase A2 — Runtime ABI specs + record constants (§5)

- [x] Add `TLS_LISTEN_SPEC`, `TLS_ACCEPT_SPEC`, and the `TlsListener` layout consts.

Acceptance: the specs are registered in the `RuntimeHelper::Tls` table and the helper symbols resolve. DONE — `TLS_LISTEN_SPEC`/`TLS_ACCEPT_SPEC`/`TLS_CLOSE_LISTENER_SPEC` in `net_specs.rs` + catalog; `TlsListener` record `{fd@0, closed@8, ctx@16, reserved@24}` in `tls/mod.rs`.

### Phase A3 — Linux OpenSSL backend (§6)

- [x] Implement listen, accept, and the shared-context null-guarded close.

Acceptance (runtime proof): on Linux aarch64, an `mfb`-built server (`tls::listen` + `tls::accept` + `tls::writeText`) serves a line that both (a) `openssl s_client -connect` and (b) the `mfb` `tls::connect` client read after a valid handshake with a self-signed test cert; client `tls` goldens remain byte-identical. DONE — cross-validated on ArchLinux glibc + Alpine musl (aarch64) *and* Alpine + Ubuntu (x86_64, which reuse the same shared OpenSSL lowering) with `openssl s_client` (both directions) and the `mfb` client. Null-guarded close (accepted socket's ctx slot = 0) + separate `tls.closeListener` body confirmed by a multi-accept / sibling-close / listener-close-while-live drop-order test. Full acceptance suite: 981 tests green.
