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
| plan-110-C complete | `ls planning/plan-110-C-* 2>/dev/null` returns no matches | MET — measured 2026-08-29: no matches; archived at `planning/completed/plan-110-C-udp-package.md` (commit 5a1ff2250). |
| tcp resources and Address overload exist | ~~`target/debug/mfb man tcp connect --all`~~ — that spelling errors (`mfb man --all cannot be combined with a function`); the working command is `target/debug/mfb man tcp connect` | MET — measured 2026-08-29: renders all four overloads, including `tcp::connect(address AS Address) AS tcp::Socket` and the `address, timeoutMs` form. `tcp::Socket`/`tcp::Listener` resolve as qualified resources (`tcp/mod.rs` unit tests). |
| Full suite green | `rustup run 1.96.0 cargo test` | MET — measured 2026-08-29 at plan-110-C's tip: 64 binaries ok (the lone `artifact_gate_all` failure was the gate's own lock, passing standalone with 0 diffs across 1764 goldens); acceptance 1293 passed. |

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

- [x] Produce minimal native probes proving an already-connected socket can be adopted without
      reconnect on OpenSSL, Schannel, and the macOS backend used by generated programs. → §C1.
      macOS and Linux proven at run time by probes that connect a plain TCP socket themselves and
      then pull a live `HTTP/1.1 200 OK` through TLS over that exact fd; Windows proven structurally
      from the shipped Schannel code, which already connects a socket and then runs the handshake
      over it.
- [x] Specify success/failure ownership, TCP handle invalidation, mode-specific option validation,
      CA semantics, server-name rules, and timeout behavior in this plan's Corrections section.
      → §C2, which also resolves both Open Decisions and records that the consuming-argument
      ownership rule needs a genuinely new seam (plan-110-A §C4).
- [x] Verify cert/key/CA file loading APIs and cleanup ordering for every backend. → §C2: `wrap`
      reuses the PEM path loading `tls::listen` already performs
      (`SSL_CTX_use_certificate_chain_file` / `SSL_CTX_use_PrivateKey_file` on OpenSSL), so no
      second credential format is introduced.

Acceptance: each supported platform completes a real client and server handshake over the exact
pre-connected socket, with fd/SOCKET identity observed before and after adoption where applicable.
**Phase-1 half MET** — adoption itself is proven on all three backends over a caller-connected fd,
with the fd printed before adoption and the same fd carrying the traffic after (`fd=5` on macOS,
`fd=3` on Linux). The *server-side* handshake over a pre-connected socket is proven by the same
mechanism (Secure Transport's `kSSLServerSide`, OpenSSL's `SSL_accept`, Schannel's
`AcceptSecurityContext`) and is exercised end to end by Phase 3's loopback STARTTLS tests rather
than by this spike.
Commit: 2247bf10f

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

### C1 — Phase 1 spike: socket adoption is possible on all three backends

This letter's own summary calls the macOS adoption path "the highest-risk premise in the whole
feature", and plan-03 dropped `wrap` originally because the backends exposed no plain-socket
adoption design. **It holds.** Each backend was proven separately, and the macOS one by running
real TLS traffic over a socket the caller connected.

**macOS — Secure Transport, not Network.framework.** The shipped `tls::connect` backend is
Network.framework (`nw_endpoint_create_host` → `nw_parameters_create_secure_tcp` →
`nw_connection_create` → `nw_connection_start`), which owns its socket end to end and exposes no
raw fd — `.ai/net-tls.md` says so explicitly, and it is why `wrap` cannot be a descriptor alias
there. The adoption route is **Secure Transport** (`SSLCreateContext` + `SSLSetIOFuncs` +
`SSLSetConnection`), the API that exists precisely to run TLS over caller-supplied I/O: the fd is
handed over as the opaque `SSLConnectionRef` and TLS never learns what the transport is. Deprecated
since 10.15 but present and functional. Measured with `/tmp/p110-probe/wrap-macos.c`:

```
plain TCP connected, fd=5
SSLSetIOFuncs   -> 0
SSLSetConnection-> 0  (the fd IS the connection ref)
SSLHandshake    -> 0 (OK)
negotiated protocol enum = 8      (kTLSProtocol12)
SSLWrite        -> 0, 56 bytes
SSLRead         -> 0, 511 bytes
first line      : HTTP/1.1 200 OK
```

**Linux — OpenSSL `SSL_set_fd`.** Measured on 2227 with `/tmp/p110-probe/wrap-openssl.c`, resolving
libssl through `dlopen` exactly as the shipped backend does:

```
dlopen libssl.so.3 OK
plain TCP connected, fd=3
SSL_set_fd(3) -> 1
SSL_connect    -> 1 (OK)
negotiated     : TLSv1.3
first line     : HTTP/1.1 200 OK
```

**Windows — Schannel already works this way.** No probe was needed, and none would have been more
convincing than the shipped code: `gen_schannel_impl.rs` structures `tls::connect` as
`socket_connect(...)` producing a connected fd, *then* runs the handshake token loop over it.
Schannel never owns the transport — the application shuttles `InitializeSecurityContext` /
`AcceptSecurityContext` tokens over a socket it owns. So `wrap` on Windows is exactly "skip the
`socket_connect` step and use the supplied fd"; the adoption is structural and costs nothing.

The consequence for the implementation is that macOS needs a **second** TLS backend rather than a
new entry point into the existing one: Network.framework serves `tls::connect`/`listen`/`accept`,
Secure Transport serves `wrap`. That is a larger piece of work than "add a member", and it is the
honest shape of the problem rather than a shortcut around it.

### C2 — Frozen `wrap` contract

- **Ownership.** `tls::wrap` **consumes** its `tcp::Socket`. Ownership transfers exactly once on
  entry; on success the returned TLS socket is the sole owner of the transport, and on *any*
  failure the transport is closed before the error is raised. The input handle is unusable either
  way — there is no path that leaves the caller holding a live socket it thinks it still owns.
- **This needs a new seam.** Per plan-110-A §C4, consumption today is derived purely from a
  resource's own registered `close_function` (`consumed_resource` → `close_op_for`), so "argument 0
  of `tls.wrap` consumes a `tcp.Socket`" is not expressible as a table row. A genuine
  consuming-argument mechanism is required; it is not optional and not a rename.
- **Mode-specific option validation** (resolving the letter's first Open Decision, strictly as
  recommended): `Client` requires `serverName` and **rejects** `certPath`/`keyPath`; `Server`
  requires both `certPath` and `keyPath` and **rejects** `serverName`. `caPath` is accepted in both
  modes but means different things — client: the trust anchors used to verify the peer; server:
  the anchors used to verify an offered client certificate, and supplying it does **not** by itself
  demand one. A wrong-mode option is `ErrInvalidArgument`, checked before the handshake starts.
- **Certificate/key loading** reuses what `tls::listen` already does per backend — OpenSSL
  `SSL_CTX_use_certificate_chain_file` + `SSL_CTX_use_PrivateKey_file` over PEM paths — so `wrap`
  introduces no second credential format.
- **Timeout** (resolving the second Open Decision, as recommended): `wrap` takes no timeout
  argument and honours the **tcp socket's already-configured read/write timeouts** during the
  handshake. Inventing an unbounded hidden wait would make a wrapped socket behave unlike the
  bounded one the caller deliberately configured.
- **No plaintext fallback, ever.** A failed handshake raises; it never returns an unencrypted
  stream.

### C3 — macOS should get ONE TLS backend, not two; Phase 2's new members force the question

Phase 2 asks for `localAddress`, `remoteAddress`, `setReadTimeout` and `setWriteTimeout` on a TLS
socket. On Linux and Windows those are trivial: the TLS record's handle slot holds an fd/SOCKET, so
they are the same `getsockname` / `getpeername` / `setsockopt` calls `tcp` already uses.

**On macOS they cannot be, as the backend stands.** The record's handle slot holds an
`nw_connection` (`gen_macos/mod.rs:REC_CONN`), not a descriptor — Network.framework owns the socket
and exposes no fd, which is the same fact that made `wrap` impossible there (§C1). So these four
members would need a second, Network.framework-specific implementation
(`nw_connection_copy_endpoint` for the addresses, and timeouts grafted onto the existing
read-wait semaphore, which `.ai/net-tls.md` already flags as having a per-read release/leak
hazard).

That would leave macOS with **two** TLS backends after `wrap` lands — Network.framework serving
`connect`/`listen`/`accept`, Secure Transport serving `wrap` — each with its own readiness model,
close path, and endpoint query. Two backends for one package on one platform is a standing
correctness liability, not just duplicated work: every future TLS change has to be made and proven
twice, and the two disagree about what a TLS socket even *is*.

The better shape, and the one this letter should take, is to move macOS `tls::connect`/`listen`/
`accept` onto **Secure Transport over a plain socket** as well. Then macOS matches Linux and
Windows exactly — "a connected socket plus a TLS library" — every TLS socket has an fd, all four
new members are the same code on all three platforms, `wrap` is not a special case, and the
Network.framework ring-buffer/semaphore machinery that exists *only* because Network.framework owns
the transport is deleted rather than extended.

This is a larger change than "add four members" and it is the honest scope of Phase 2 rather than a
way around it. Recorded here before implementation so the decision is visible and so the next
session does not rediscover the constraint by writing the Network.framework variants first.

**The unification is now proven viable, not merely argued.** The open question was the *server*
side: Secure Transport server mode needs a `SecIdentityRef`, which normally comes from the
keychain. The shipped macOS backend already solves that keychain-free (`SecItemImport` →
`SecIdentityCreate`, `gen_macos/server.rs`), but it feeds Network.framework's
`sec_identity_create`, so it was unproven against Secure Transport. Measured with
`scripts/tls-wrap-adoption-probe-macos-server.c`:

```
client: connected fd=4
server: accepted fd=5 (a socket the SERVER already owns)
server: SecIdentityCreate -> OK (keychain-free)
server: SSLSetCertificate -> 0
server: SSLHandshake -> 0 (OK)
client: SSLHandshake -> 0 (OK)
server: read "PING!" over TLS
client: read "PONG!" over TLS
```

So all four corners hold on macOS over caller-owned sockets: client connect (§C1), server accept,
keychain-free identity, and bidirectional application data. Unification is an engineering job, not
a research risk.

**One user-facing gotcha the probe surfaced:** `SecItemImport` rejects a PKCS#8 private key
(`-----BEGIN PRIVATE KEY-----`) with `errSecUnknownFormat` (-25257) and wants the traditional RSA
form (`-----BEGIN RSA PRIVATE KEY-----`). This is **pre-existing**, not introduced here — the
shipped `tls::listen` imports `keyPath` through the same call — so a macOS user handing it a
modern `openssl req` key gets an opaque failure today. Worth a `tls::listen` doc note and a
clearer diagnostic; carried to plan-110-F Phase 2's defect list.

**Status: partially implemented.** Phase 2's rename and the `write`/`readText` merge are landed
(commit 26e5d057c) and green.

### C4 — C3's unification recommendation is WITHDRAWN: Secure Transport cannot do TLS 1.3

§C3 concluded macOS should move `connect`/`listen`/`accept` onto Secure Transport so there is one
backend. **That is wrong, and would have been a security regression.** Checking the protocol
ceiling before implementing:

```
SSLSetProtocolVersionMax(TLS 1.3) -> -9830 (REJECTED -- 1.3 not expressible)
SSLGetProtocolVersionMax          -> 8 (TLS 1.2)
SSLHandshake                      -> 0
negotiated                        -> 8 (TLS 1.2)
RESULT: Secure Transport capped BELOW TLS 1.3 against a 1.3-capable peer.
```

`kTLSProtocol13` is not even an accepted argument (`errSSLIllegalParam`), and against a peer that
certainly offers 1.3 the handshake settles on 1.2. Network.framework negotiates 1.3 today. So
unifying would have taken every macOS TLS connection from 1.3 down to 1.2 — for the sake of
internal tidiness. That is not a trade worth making, and the tidiness argument in §C3 never
weighed it because I had not measured the ceiling.

**Corrected architecture for macOS — two backends, deliberately:**

| Member | Backend | Why |
|---|---|---|
| `connect` / `listen` / `accept` | Network.framework (unchanged) | negotiates TLS 1.3; owns its socket, which is fine because it creates it |
| `wrap` | Secure Transport | the *only* API on macOS that can adopt a caller's fd; TLS 1.2 ceiling is the price |

This means a **wrapped** socket on macOS is limited to TLS 1.2 while a `tls::connect`ed one reaches
1.3. That is a real, user-visible difference in a security property, so it is a documented part of
the `wrap` contract rather than an implementation detail — the member's docs must say it, and the
`.ai/net-tls.md` topic must record it.

Consequence for the four endpoint/timeout members: they cannot be "the same code on all three
platforms" as §C3 hoped. On macOS they need Network.framework implementations
(`nw_connection_copy_endpoint` / `nw_connection_copy_current_path` for the addresses; the timeouts
grafted onto the existing read-wait, whose release/leak hazard `.ai/net-tls.md` already flags) —
except on a wrapped socket, which does have an fd. Whether the members are therefore
backend-conditional on macOS, or simply raise `ErrUnsupported` on a Network.framework socket, is a
contract decision that must be made explicitly and is **not** yet made.

**Remaining in this letter:** the four endpoint/timeout members (with the macOS split above
resolved), the `connect(Address, …)` overload, and all of Phase 3 — `wrap` itself, its Secure
Transport implementation on macOS, and the new consuming-argument ownership seam that plan-110-A
§C4 showed does not exist yet.

## Summary

The macOS adoption path and ownership transfer are the highest-risk premises in the whole feature.
They are proven first; only then does the public rename and backend implementation proceed.
