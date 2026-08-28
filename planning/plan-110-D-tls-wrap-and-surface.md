# plan-110-D: TLS wrap and renamed resource surface

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-110-C

Rename TLS resources to `tls::Socket`/`tls::Listener`, add Address overloads and endpoint/timeout
members, and implement `tls::wrap` over an existing `tcp::Socket` for both client and server mode.

References: plan-110-C; `.ai/net-tls.md`; `.ai/arch-abi.md`;
`src/codegen/builtins/tls/{mod.rs,gen_shared.rs,gen_openssl.rs,gen_schannel.rs,gen_macos/}`;
`planning/completed/plan-03-net.md`; `planning/completed/plan-06-tls-server.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-110-C complete | `ls planning/plan-110-C-* 2>/dev/null` returns no matches | NOT MET |
| tcp resources and Address overload exist | `target/debug/mfb man tcp connect --all` | NOT MET |
| Full suite green | `rustup run 1.96.0 cargo test` | UNVERIFIED |

## 1. Goal

- Deliver every requested tls signature, including real client/server wrapping of an established
  tcp connection, correct certificate/trust behavior, address queries, poll, and timeouts.

### Non-goals

- No plaintext fallback after TLS failure; no disabled certificate verification unless the
  explicit CA/certificate contract says so.
- Do not expose backend-specific OpenSSL/Schannel/Network.framework handles.
- Do not keep `readText`/`writeText`; String is the second `write` overload.
- Do not simulate wrap by reconnecting: it must use the exact supplied TCP stream.

## 2. Current State

TLS has 22 implementation files (`find src/codegen/builtins/tls -type f | wc -l`) and currently
registers `TlsSocket`, `TlsListener`, connect/listen/accept/read/readText/write/writeText/poll/close
(`src/codegen/builtins/tls/mod.rs:register`). It has no wrap, local/remote address, or timeout
setters. Completed plan 03 explicitly dropped wrap because the original backends did not expose a
plain-socket adoption design; this plan must solve that premise rather than restore dead metadata.

Verified: OpenSSL and Schannel ultimately own an OS socket, but macOS Network.framework has no raw
fd (`.ai/net-tls.md`, `src/codegen/builtins/tls/gen_macos/`). Therefore wrap is a new backend design,
not a descriptor alias. TLS readiness must check decrypted buffered bytes before transport
readiness.

## 3. Design Overview

Rename descriptor identities and docs while keeping runtime record layouts stable where possible.
`tls::wrap` consumes `tcp::Socket`: ownership transfers exactly once on entry; success returns the
sole TLS owner, and any failure closes the transport before raising Error. Extend builtin argument
ownership metadata so only wrap's first argument and tls close consume.

Client mode uses `serverName` (required in practice for DNS certificate validation unless the
peer is intentionally validated another documented way), optional `caPath`, and must reject
server-only cert/key inputs. Server mode requires certPath+keyPath, may use caPath for client trust
only if the chosen mutual-TLS contract explicitly enables it, and rejects client-only serverName.
Backend implementations must adopt the supplied connected transport: OpenSSL `SSL_set_fd`,
Schannel over the existing SOCKET, and a proven Network.framework connection/adoption route. Phase
1 is an uncertainty spike; lack of a real macOS route is a prerequisite blocker, never license for
a reconnect or unsupported stub.

This intentionally changes TLS package metadata and all TLS/HTTP fixtures on every target. Backend
handshake bodies also legitimately change for wrap and Address overloads.

## Phases

### Phase 1 — Prove wrap ownership on all backends

- [ ] Produce minimal native probes proving an already-connected socket can be adopted without
      reconnect on OpenSSL, Schannel, and the macOS backend used by generated programs.
- [ ] Specify success/failure ownership, TCP handle invalidation, mode-specific option validation,
      CA semantics, server-name rules, and timeout behavior in this plan's Corrections section.
- [ ] Verify cert/key/CA file loading APIs and cleanup ordering for every backend.

Acceptance: each supported platform completes a real client and server handshake over the exact
pre-connected socket, with fd/SOCKET identity observed before and after adoption where applicable.
Commit: —

### Phase 2 — Resource rename and existing operations

- [ ] Rename public resources to `tls.Socket`/`tls.Listener`; update registry, verifier, cleanup,
      resource tags, tests, aliases, diagnostics, and docs without changing layout accidentally.
- [ ] Provide host and `net.Address` connect overloads; add localAddress/remoteAddress and read/write
      timeout setters; collapse String write into `tls::write`.
- [ ] Preserve omitted timeout as unbounded and `0` as immediate per `.ai/net-tls.md`.
- [ ] Tests: every overload, wrong types/arity, endpoints, timeout conventions, close/drop, and
      certificate-name verification.

Acceptance: existing direct client/server workflows run under the new types and exact requested
signatures on all target families.
Commit: —

### Phase 3 — Implement wrap

- [ ] Add `WrapMode { Server, Client }`, the wrap descriptor/defaults, consuming ownership rule,
      and per-backend adopt/handshake/cleanup code.
- [ ] Add loopback STARTTLS-style runtime tests: establish tcp, exchange a plaintext preface, wrap
      both ends, exchange encrypted bytes/String, query addresses, poll, timeout, and close.
- [ ] Add negative runtime cases: denied/missing cert/key/CA, key mismatch, untrusted CA, hostname
      mismatch, invalid mode-option combinations, closed/unconnected tcp socket, and handshake
      failure; prove the input resource cannot be reused or double-closed.

Acceptance: both modes handshake over the supplied connection and all negative cases raise the
documented Error with leak/double-close checks green.
Commit: —

## Validation Plan

Run the full Rust suite, tls syntax/rt-behavior/rt-error fixtures, acceptance, local macOS TLS
runtime tests, Linux glibc/musl/riscv64 runtime proof, Windows codegen/runtime proof, and artifact
gates. Preserve the decrypted-buffer readiness invariant. Update man descriptors and the embedded
stdlib/error specs. Run both required rustfmt commands.

## Open Decisions

- Mode-option validity — recommend strict validation: Client rejects cert/key; Server requires
  cert+key and rejects serverName. Define caPath separately for client trust and optional server
  client-auth before coding.
- Wrap timeout — the requested signature has none; recommend honoring the tcp socket's configured
  read/write timeouts during handshake rather than inventing an unbounded hidden wait.

## Corrections

To be filled during execution.

## Summary

The macOS adoption path and ownership transfer are the highest-risk premises in the whole feature.
They are proven first; only then does the public rename and backend implementation proceed.
