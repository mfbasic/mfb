# MFBASIC TLS Server Plan — overview

Last updated: 2026-06-30
Overall Effort: large

**Split plan** (by effort into two small/medium sub-plans; see §Sub-plans). This file is the
overview holding the shared design; the phases live in the lettered sub-plans.

This plan adds the **server side** to the `tls` package. Today `tls` is a client
only: `tls::connect` opens an outbound, certificate-verified TLS stream, but there
is no way to *terminate* TLS — to bind a port, present a server certificate, and
accept encrypted inbound connections. `net` already has the plaintext server spine
(`net::listenTcp` / `net::accept`) but offers no encryption. This plan closes the
gap by adding `tls::listen` (bind + load the server identity) and `tls::accept`
(accept one client and complete the server-side handshake), returning a normal
`TlsSocket` that reads and writes with the existing `tls::read`/`tls::write`
family. A correct implementation lets a pure-MFBASIC program terminate TLS: an
`mfb`-built server presents a certificate that the existing `tls::connect` client
(and `openssl s_client`, `curl`) validates and exchanges application data over.

It complements:

- `./mfb man tls` and `./mfb man net` (the two package surfaces this unifies; the
  canonical man sources live under `src/docs/man/builtins/{tls,net}/**`)
- `./mfb spec diagnostics error-codes` (the `Err*` table reused for server errors;
  canonical spec under `src/docs/spec/**`)
- `planning/old-plans/plan-03-net.md` §4 (the original `tls` client design this
  mirrors) and `planning/plan-05-http-server.md` (the primary downstream consumer —
  an HTTPS listener needs this)

## 1. Goal

- Add two built-in functions and one built-in resource type to `tls`:
  - `tls::listen(host AS String, port AS Integer, certPath AS String, keyPath AS String) AS TlsListener`
    — resolve a local endpoint, `bind`/`listen` a TCP socket, and load the PEM
    certificate chain + private key into a **server** TLS context.
  - `tls::listen(host, port, certPath, keyPath, backlog AS Integer) AS TlsListener`
    — as above with a backlog hint (mirrors `net::listenTcp`).
  - `tls::accept(listener AS TlsListener) AS TlsSocket` and
    `tls::accept(listener AS TlsListener, timeoutMs AS Integer) AS TlsSocket`
    — accept one inbound TCP connection and complete the **server-side** TLS
    handshake, returning a connected `TlsSocket`.
  - `TlsListener` — a new opaque, owned, non-copyable resource handle (closed by
    lexical drop or `tls::close`).
- The returned `TlsSocket` is byte-for-byte interchangeable with a client
  `TlsSocket`: `tls::read`/`tls::readText`/`tls::write`/`tls::writeText`/`tls::close`
  work unchanged on it.
- Runtime proof on **Linux (OpenSSL)** and **macOS (Network.framework)**: an
  `mfb`-built `tls::listen`/`tls::accept` server and an `mfb`-built `tls::connect`
  client complete a handshake and exchange a message in both directions.

### Non-goals (explicit constraints)

- **No language-surface change.** No new keywords, statements, or type syntax; `tls`
  stays a native built-in package registered exactly like `net`/`tls` today. No
  change to value/copy/move/freeze semantics.
- **No change to the existing client path.** `tls::connect` and the read/write/close
  helpers, their ABI, their record layout (Linux `{fd@0, closed@8, SSL*@16,
  SSL_CTX*@24}`), and their golden output must be untouched. Adding server helpers
  must not perturb any existing `.ncode`/native golden.
- **Resources stay non-sendable and non-collectable.** `TlsListener` and the
  server-accepted `TlsSocket` are **not** thread-sendable in V1 (same as the client
  `TlsSocket`, `net::Listener`), and cannot be stored in a `List` or a record.
- **No mutual TLS (client-certificate verification) in V1.** The server presents its
  certificate; it does not request or verify a client certificate.
- **No ALPN, no session resumption/tickets, no SNI-based cert selection.** One
  identity per listener.
- **No new thread-transfer rules, no layout/ABI change to any existing type.**

## 2. Current State

### `tls` is client-only

`src/builtins/tls.rs` defines exactly six calls — `CONNECT`, `READ`, `READ_TEXT`,
`WRITE`, `WRITE_TEXT`, `CLOSE` (`tls.rs:14-19`) — and one built-in type `TlsSocket`
(`tls.rs:12`). The man page states it plainly: *"The package is a client only —
there is no listener or accept side; use net for plain unencrypted TCP and UDP."*
(`src/docs/man/builtins/tls/package.txt`).

The `TlsSocket` resource is registered in `src/builtins/resource.rs:162-173`:
`close_function = tls.close`, `sendable = false`, `close_may_fail = true`,
`kind = Builtin`.

### Linux backend (OpenSSL)

`src/target/shared/code/tls/mod.rs` documents the client record layout — a 32-byte
arena record `{fd@0, closed@8, SSL*@16, SSL_CTX*@24}` (`mod.rs:19-25`) — and the
`dlopen`/`dlsym` model: each helper re-`dlopen`s `libssl.so.3` (falling back to
`libssl.so.1.1`) and `dlsym`s the `SSL_*` symbols it needs (`mod.rs:40-60`). The
current symbol table (`TLS_SYMBOLS`, `mod.rs:44-60`) is entirely **client**-shaped:
`TLS_client_method`, `SSL_connect`, `SSL_get_verify_result`, `SSL_set1_host`, etc.
The connect/read/write/close helpers are lowered in
`src/target/shared/code/tls/openssl.rs`.

### macOS backend (Network.framework)

`src/target/shared/code/tls/macos.rs` builds Objective-C block literals and drives
`nw_connection` through a `dispatch_semaphore` synchronous bridge; the AArch64 block
trampolines live in `src/target/macos_aarch64/tls.rs`. The symbol list
(`macos.rs:57-82`) is client-only (`nw_connection_*`,
`nw_parameters_create_secure_tcp`); there is **no** `nw_listener_*` and no
`sec_identity`/`sec_protocol_options` identity plumbing yet.

### Runtime ABI specs

`src/target/shared/runtime/net_specs.rs:483-547` declares the six
`RuntimeHelperSpec`s (`TLS_CONNECT_SPEC` … `TLS_CLOSE_SPEC`) with fixed parameter
layouts (`TLS_CONNECT_PARAMS`, `TLS_SOCKET_INT_PARAMS`, etc.). Helper symbols follow
`_mfb_rt_tls_tls_<call>`.

### The precedent to mirror: `net` server spine

`net::listenTcp` lowers via `lower_net_endpoint_helper(..., listen=true)`
(`src/target/shared/code/net/mod.rs:744-750`, body `278-750`): resolve host (NULL +
`AI_PASSIVE` for all interfaces), create socket, `SO_REUSEADDR`, `bind`, `listen`.
`net::accept` (`src/target/shared/code/net/io.rs:16-99`) calls `accept(fd, NULL,
NULL)` and allocates a fresh `Socket` record from the accepted fd via
`emit_make_handle`. `net::Listener` and `net::Socket` share the `File` layout
`{fd@0, closed@8}`. Specs: `NET_LISTEN_TCP_SPEC`, `NET_ACCEPT_SPEC`
(`net_specs.rs:171-192`).

## 3. Design Overview

The design is deliberately symmetric with `net`'s server spine, plus one new piece
of state — the **server TLS context / identity** — that `tls::listen` builds once
and every `tls::accept` reuses.

Four independent layers, lowest-risk first:

1. **Front-end surface** (`src/builtins/tls.rs`, `resource.rs`): add `LISTEN` /
   `ACCEPT` calls, the `TlsListener` type, overloads, arity, argument defaulting.
   Pure declaration; type-checks with no codegen.
2. **Runtime ABI specs** (`net_specs.rs`): `TLS_LISTEN_SPEC`, `TLS_ACCEPT_SPEC` and
   their parameter layouts, plus the `TlsListener` record-layout constants in
   `tls/mod.rs`.
3. **Linux OpenSSL backend** (`tls/openssl.rs`): `lower_tls_listen_helper` (server
   `SSL_CTX` + PEM cert/key load + bind/listen) and `lower_tls_accept_helper`
   (`accept` + `SSL_new`/`SSL_set_fd`/`SSL_accept`). **This is where correctness
   concentrates** — the *shared-context ownership* rule (below).
4. **macOS Network.framework backend** (`tls/macos.rs`, `macos_aarch64/tls.rs`):
   `nw_listener` + a `sec_identity` built from the cert/key. **This is the highest
   risk** — Network.framework wants a `SecIdentity`, not PEM files (see Open
   Decisions).

The correctness crux common to both backends: **the server context/identity is
owned by the `TlsListener`, and each accepted `TlsSocket` borrows it.** An accepted
socket must *not* free the shared server context on `tls::close` (that would
double-free / kill live siblings). The listener frees it exactly once when the
listener drops.

## 4. Front-end surface (`src/builtins/tls.rs`, `resource.rs`)

Add call constants and a type:

```rust
const LISTEN: &str = "tls.listen";
const ACCEPT: &str = "tls.accept";
pub(crate) const TLS_LISTENER_TYPE: &str = "TlsListener";
```

Wire them through every dispatch point that `CONNECT` already flows through:

- `is_tls_call` (`tls.rs:26-31`): add `LISTEN | ACCEPT`.
- `is_builtin_type` (`tls.rs:33-35`): accept `TlsListener` as well as `TlsSocket`.
- `resource_close_function` (`tls.rs:37-39`): `TlsListener` → `tls.close`
  (`tls::close` becomes overloaded over both handle types — see §4.1).
- `call_param_names` (`tls.rs:41-50`):
  - `LISTEN` → `[["host"], ["port"], ["certPath"], ["keyPath"], ["backlog"]]`
  - `ACCEPT` → `[["listener"], ["timeoutMs"]]`
- `call_return_type_name` / `resolve_call` (`tls.rs:52-80`):
  - `LISTEN` with `[String, Integer, String, String]` or
    `[String, Integer, String, String, Integer]` → `TlsListener`
  - `ACCEPT` with `[TlsListener]` or `[TlsListener, Integer]` → `TlsSocket`
- `expected_arguments` / `argument_types` (`tls.rs:82-102`): add the two calls.
- `arity` (`tls.rs:104-111`): `LISTEN => (4, 5)`, `ACCEPT => (1, 2)`.
- `default_argument_padding` (`tls.rs:116-126`): `ACCEPT` pads `timeoutMs := 0`;
  `LISTEN` pads `backlog := 0` (0 ⇒ host default, matching `net::listenTcp`).
- `consumes_argument` (`tls.rs:130-132`): `tls.close` already consumes arg 0; that
  now applies to a `TlsListener` operand too (no change needed beyond the type
  flowing through). `tls.accept` does **not** consume its listener (the listener
  stays open for the next accept — mirror `net::accept`).

Register the new resource in `src/builtins/resource.rs` next to `TlsSocket`
(`resource.rs:162-173`):

```rust
entries.insert(TLS_LISTENER_TYPE, ResourceInfo {
    close_function: "tls.close",
    sendable: false,          // not thread-sendable in V1
    close_may_fail: true,
    kind: ResourceKind::Builtin,
});
```

### 4.1 `tls::close` over two handle types

`tls::close` must accept either `TlsSocket` or `TlsListener`. Two options: (a) one
`tls.close` call whose lowering branches on the operand's record shape, or (b) a
distinct `tls.closeListener` call. Recommend **(a)** — keep the single documented
`tls::close`, add the `[TlsListener]` overload to `resolve_call`, and have the close
lowering distinguish the two records by their layout tag (see §6 close semantics).
This matches how `net::close` already spans `Socket`/`Listener`/`UdpSocket`.

## 5. Runtime ABI specs & record layout

### 5.1 `TlsListener` record layout (`tls/mod.rs`)

The listener needs the listening fd plus the server context. Reuse a 32-byte arena
record, distinct offsets from the `TlsSocket` record so close can tell them apart if
needed:

```
TlsListener (Linux): { fd@0, closed@8, SSL_CTX*@16 (server ctx, owned), reserved@24 }
```

Accepted `TlsSocket` records keep the existing `{fd@0, closed@8, SSL*@16,
SSL_CTX*@24}` **but store 0 in the `SSL_CTX*` slot** — the marker that "this socket
does not own its context" (the shared server ctx is owned by the listener). The
existing close helper already `SSL_CTX_free`s slot 24; it must be made to **skip the
free when slot 24 is 0** (a null-guard — see §6). Client sockets continue to store
their per-connection ctx there and free it, so client behavior is byte-identical.

### 5.2 Specs (`net_specs.rs`)

Add alongside the existing TLS specs (`net_specs.rs:483-547`):

- `TLS_LISTEN_SPEC` — `call: "tls.listen"`, `symbol:
  "_mfb_rt_tls_tls_listen"`, params `TLS_LISTEN_PARAMS` (host @x0 String, port @x1
  Integer, certPath @x2 String, keyPath @x3 String, backlog @x4 Integer),
  `returns: "TlsListener"`.
- `TLS_ACCEPT_SPEC` — `call: "tls.accept"`, `symbol: "_mfb_rt_tls_tls_accept"`,
  params `TLS_ACCEPT_PARAMS` (listener @x0 TlsListener, timeoutMs @x1 Integer),
  `returns: "TlsSocket"`.

Register both in the `RuntimeHelper::Tls` dispatch table wherever the six existing
TLS specs are enumerated.

## 6. Linux OpenSSL backend (`tls/openssl.rs`, `tls/mod.rs`)

### 6.1 New symbols

Extend `TLS_SYMBOLS` (`tls/mod.rs:44-60`) with the server-side entry points:

```
TLS_server_method,
SSL_CTX_use_certificate_chain_file,
SSL_CTX_use_PrivateKey_file,
SSL_CTX_check_private_key,
SSL_accept,
```

`SSL_CTX_new`, `SSL_new`, `SSL_set_fd`, `SSL_read`, `SSL_write`, `SSL_shutdown`,
`SSL_free`, `SSL_CTX_free` are already present and reused.

### 6.2 `lower_tls_listen_helper`

1. Build the listening socket exactly as `net::listenTcp` does — resolve
   host/port (NULL + `AI_PASSIVE` for `""`/`0.0.0.0`), `socket`, `SO_REUSEADDR`,
   `bind`, `listen(fd, backlog?)`. Reuse the `net` endpoint-lowering path rather
   than re-deriving it (call into the shared `net` helper, or factor the
   bind/listen sequence so both packages share it). Errors map to the same codes as
   `net::listenTcp`: `ErrAddressInvalid` (77070001), `ErrNetworkFailed` (77070003).
2. `SSL_CTX_new(TLS_server_method())`; set min proto TLS 1.2
   (`SSL_ctrl(SSL_CTRL_SET_MIN_PROTO_VERSION, TLS1_2_VERSION)` — constants already in
   `mod.rs:35-36`).
3. `SSL_CTX_use_certificate_chain_file(ctx, certPath)`,
   `SSL_CTX_use_PrivateKey_file(ctx, keyPath, SSL_FILETYPE_PEM=1)`,
   `SSL_CTX_check_private_key(ctx)`. Any failure ⇒ `SSL_CTX_free`, close fd, raise
   `ErrTlsFailed` (77070008). (certPath/keyPath are NUL-terminated arena C-strings,
   same pattern as the host copy in `connect`.)
4. Allocate the `TlsListener` record: `{fd, closed=0, ctx, reserved=0}`.

### 6.3 `lower_tls_accept_helper`

1. `accept(listener.fd, NULL, NULL)` — mirror `net::accept`
   (`net/io.rs:16-99`), including the closed-listener guard
   (`ErrResourceClosed` 77030004) and `ErrNetworkFailed` on failure.
2. `SSL_new(listener.ctx)`, `SSL_set_fd(ssl, connfd)`, `SSL_accept(ssl)` — the
   server handshake. Failure ⇒ `SSL_free`, close connfd, `ErrTlsFailed`.
3. Allocate a `TlsSocket` record `{fd=connfd, closed=0, ssl, ctx=0}` — **`ctx=0`
   marks the shared, non-owned context** (§5.1). Return it.

### 6.4 Close semantics (shared-context ownership — the crux)

Modify the existing close lowering so `SSL_CTX_free` at slot 24 is **null-guarded**:
free the `SSL*` (`SSL_shutdown` + `SSL_free`) always, then `SSL_CTX_free` only when
the ctx slot is non-zero. This makes:

- **client `TlsSocket`** (ctx ≠ 0): unchanged — frees its own ctx (byte-identical).
- **accepted `TlsSocket`** (ctx = 0): frees only the `SSL`, leaves the shared server
  ctx alone.
- **`TlsListener`**: closes its fd and frees its server ctx (slot 16) exactly once.

The listener close path differs from the socket close path (no `SSL*` to shut down;
ctx at offset 16 not 24), which is why §4.1 recommends dispatching on record shape.
Concretely, tag the two records or route `TlsListener` closes to a distinct internal
close body while keeping the single user-facing `tls::close` name.

## 7. macOS Network.framework backend (`tls/macos.rs`, `macos_aarch64/tls.rs`)

This is the highest-risk layer; treat it as the last phase, gated behind a working
Linux backend.

- **Listener:** `nw_listener_create(parameters)` where `parameters =
  nw_parameters_create_secure_tcp(...)` configured with a **`sec_identity`** via
  `sec_protocol_options_set_local_identity`. Bind the port with
  `nw_listener_set_local_endpoint` / a port on the parameters;
  `nw_listener_set_new_connection_handler` enqueues inbound `nw_connection`s;
  `nw_listener_start`. A `dispatch_semaphore` bridges the async new-connection
  callback into the synchronous `tls::accept`, exactly as the client bridges the
  state handler today (`macos.rs` block machinery).
- **Accept:** pull the next queued `nw_connection`, start it, wait for its state to
  reach `ready` (server handshake complete), wrap it in a `TlsSocket` record. Each
  accepted connection needs its own state/send/recv block context — the current
  single-context client design must generalize to per-connection contexts (called
  out as a risk in the net research).
- **Identity:** Network.framework needs a `SecIdentity`/`sec_identity_t`, **not** PEM
  files. See Open Decisions — this is the one place the cross-platform surface may
  have to bend.
- New symbols to add to `macos.rs:57-82`: `nw_listener_create`,
  `nw_listener_set_queue`, `nw_listener_set_new_connection_handler`,
  `nw_listener_set_state_changed_handler`, `nw_listener_start`,
  `nw_listener_cancel`, plus `sec_protocol_options_set_local_identity` and the
  identity-construction symbols chosen in Open Decisions.

## Layout / ABI Impact

- **New:** one resource type `TlsListener` and its 32-byte arena record layout
  `{fd@0, closed@8, ctx@16, reserved@24}`; two runtime helper symbols
  (`_mfb_rt_tls_tls_listen`, `_mfb_rt_tls_tls_accept`). Documented in `mfb spec`
  package surface for `tls`.
- **Changed (semantically, not in size):** the accepted `TlsSocket` uses the
  existing 32-byte layout but with a **0 in the `SSL_CTX*` slot**, and the close
  helper gains a null-guard on that slot. Client `TlsSocket` records and their close
  behavior are **byte-identical** to today.
- **Unchanged:** every existing type layout, the `net` package, thread-transfer
  rules (both new handles are non-sendable), and all existing `tls`/`net` goldens.

## Sub-plans

Split by effort into two small/medium sub-plans; each holds its phases, tasks, and acceptance. The
design sections above (§4–§7) are the shared source of truth both reference.

| Doc | Effort | Phases | Depends on |
|---|---|---|---|
| [plan-06-A](plan-06-A-surface-linux.md) — surface, specs, Linux backend | medium | front-end surface (§4) · ABI specs (§5) · Linux OpenSSL backend (§6) | — |
| [plan-06-B](plan-06-B-macos-docs-tests.md) — macOS backend, docs, tests | medium | macOS Network.framework backend (§7) · man/spec · tests + acceptance | A |

## Validation Plan

- **Function tests** (full overload coverage):
  - `tests/func_tls_listen_valid/**` — 4-arg and 5-arg (backlog) forms;
    `tests/func_tls_listen_invalid/**` — wrong arity, wrong types (non-String
    cert/key, non-Integer port/backlog), storing a `TlsListener` in a `List` or
    record (must be rejected — non-collectable resource).
  - `tests/func_tls_accept_valid/**` — 1-arg and 2-arg (timeout) forms;
    `tests/func_tls_accept_invalid/**` — wrong arity/types, passing a `TlsSocket`
    where a `TlsListener` is required.
  - `tests/func_tls_close_*` — extend to cover closing a `TlsListener` and confirm
    it does not double-free when accepted sockets from it are still open/closed.
- **Runtime proof** (not just golden output): the Phase 3/4 client↔server
  round-trip with a self-signed cert, in both directions (server→client and
  client→server bytes), plus a drop-order test (accept N sockets, drop them, then
  drop the listener — no double-free / no leak of the server ctx under a leak check).
- **Doc sync:** `mfb spec` `tls` package surface + `mfb spec diagnostics`
  error-codes table reflect `TlsListener`, `tls::listen`, `tls::accept`; man pages
  updated per §Phase 5.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  passes; existing `tls`/`net` native goldens unchanged.

## Open Decisions

- **Certificate/key input form** — *recommend PEM file paths*
  (`tls::listen(host, port, certPath, keyPath)`), matching OpenSSL's
  `SSL_CTX_use_certificate_chain_file` / `SSL_CTX_use_PrivateKey_file` directly and
  keeping the signature simple. *Alternative:* in-memory `List OF Byte` cert/key
  (no filesystem dependency, better for embedded secrets) — more plumbing
  (`SSL_CTX_use_certificate`/`d2i`/BIO), deferred. (§6.2)
- **macOS identity source** — the sharpest fork. Network.framework needs a
  `sec_identity_t` (a `SecIdentity` = cert + private key), **not** PEM files.
  Options: (a) *unified PEM* — on macOS, build the identity from the PEM pair via
  `SecCertificateCreateWithData` + `SecKeyCreateWithData` + `sec_identity_create`,
  so one signature works on both platforms (*recommended* — keeps the language
  surface uniform, but is the riskiest code); (b) *PKCS#12 on macOS* — accept a
  `.p12` path on macOS via `SecPKCS12Import` while Linux takes PEM (diverging
  surface — avoid); (c) *Linux-first* — land Phases 1-3 and gate macOS (Phase 4)
  behind (a) once proven. If (a) proves infeasible in the block-bridge model, fall
  back to (c) and track macOS as a follow-up — **never** ship a non-functional
  macOS stub (production-ready rule). (§7)
- **Separate `TlsListener` vs. reuse `net::Listener`** — *recommend a distinct
  `TlsListener`*: it owns the server TLS context (a plain `net::Listener` has no
  slot for it) and keeps `tls` self-contained and symmetric with the client
  `TlsSocket`. *Alternative:* `tls::accept(net::Listener)` over a plaintext listener
  — couples the packages and has nowhere to store the shared ctx. (§4, §5.1)
- **New error code for cert/key load failure?** — *recommend reuse* `ErrTlsFailed`
  (77070008) for handshake **and** identity-load failures, and the existing `net`
  bind/listen codes (`ErrAddressInvalid`, `ErrNetworkFailed`) for the socket setup —
  no new diagnostics-spec churn. *Alternative:* a dedicated `ErrCertificateLoad`
  code for clearer errors (adds a `mfb spec diagnostics` row + `errorCode::`
  rebuild). (§6.2)

## Non-Goals

- Mutual TLS / client-certificate verification, ALPN, session tickets/resumption,
  and SNI-based multi-cert selection — all out of scope for V1 (§1 Non-goals).
- Making `TlsListener` or accepted `TlsSocket` thread-sendable — deferred with the
  client `TlsSocket`'s non-sendable status.
- x86_64 backend support — `tls`/`net` native helpers are AArch64-only today
  (plan-00-H is bringing x86_64 up separately); this plan targets the two existing
  platforms (Linux aarch64, macOS aarch64) and does not add an x86_64 lowering.

## Summary

The real engineering risk is in two places. First, the **shared-context ownership**
rule on Linux: the server `SSL_CTX` is owned by the `TlsListener` and *borrowed* by
every accepted `TlsSocket`, so the close path must null-guard the socket's ctx slot
to avoid a double-free — get this wrong and it's a layout-sensitive crash under
concurrent connections. Second, the **macOS identity path**: Network.framework wants
a `SecIdentity`, not PEM, so the cross-platform `tls::listen` signature hinges on
building an identity from PEM on macOS (Open Decision, recommended but risky; a
Linux-first landing de-risks it). Everything else mirrors proven precedent — the
`net::listenTcp`/`net::accept` socket spine and the existing `tls` client backend —
and the plan leaves the entire client path, all existing layouts, and every current
golden **untouched**.
