# plan-06-B: TLS server — macOS backend, docs, tests

Last updated: 2026-06-30
Effort: medium

Part **B** of plan-06 (TLS Server). Adds the macOS Network.framework backend over the same seam,
then the man pages, spec sync, and tests that close the feature. Shared design lives in the
overview: [plan-06-tls-server.md](plan-06-tls-server.md).

- **Depends on:** plan-06-A (surface + ABI specs; the macOS backend implements the same seam).
- **Spec/design:** overview §7 (macOS Network.framework backend), Validation Plan.

## Phases

### Phase B1 — macOS Network.framework backend (§7)

- [x] Implement `nw_listener` + `sec_identity` with per-connection contexts.

Acceptance (runtime proof): the same client/server round-trip as plan-06-A Phase A3, on macOS aarch64. DONE — `tls::listen` builds a `sec_identity` from the PEM pair via Security.framework (`SecItemImport` + `SecIdentityCreate` + `sec_identity_create`) installed on an `nw_listener` through the identity-config block; a single-producer/single-consumer ring in the listener context (fed by the `LCONN_INVOKE` new-connection trampoline) hands inbound connections to `tls::accept`, which runs each server handshake on a per-connection context. Box-verified: the macOS `mfb` server ↔ `openssl s_client` round-trip in both directions, plus the multi-accept / drop-order crux. (The macOS `mfb` *client* validating a self-signed cert needs a system-trust entry — an environment step unchanged by this plan; the client path is untouched and is proven mfb↔mfb on Linux.)

### Phase B2 — Man pages + spec sync

- [x] New `src/docs/man/builtins/tls/{listen,accept}.md` (per `.ai/man_template.md`).
- [x] Update `tls/package.md` (removed "client only"; added the server synopsis and the `TlsListener` type) and `close.md` (the `TlsListener` overload).
- [x] Reused error codes only (no new diagnostics rows): `ErrTlsFailed`, `ErrAddressInvalid`, `ErrNetworkFailed`, `ErrResourceClosed`, `ErrTimeout`, `ErrOutOfMemory`. Spec surface list (`language/18_builtin-functions.md`) updated; `closeListener` marked internal and filtered from `list_functions.py`.

Acceptance: the man pages render (`mfb man tls listen` / `accept` / `close` / `tls`) and the spec reflects the server surface + reused error codes. DONE.

### Phase B3 — Tests + acceptance

- [x] Function tests: `func_tls_{listen,accept}_{valid,invalid}` (every overload) + `func_tls_close_valid` and extended `func_tls_close_invalid` for the `TlsListener` overload.
- [x] `scripts/test-accept.sh` green.

Acceptance: the function tests and the acceptance suite pass. DONE — full suite 981 tests green; the 4 remaining `cargo test` failures (x86_64 encode, math man-page rendering) are pre-existing at HEAD and unrelated.
