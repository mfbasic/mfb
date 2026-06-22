# MFBASIC Net Package Completion Plan

Last updated: 2026-06-21

This document plans the work to finish the `net` package specified in
`specifications/standard_package.md` §11. TCP and DNS are implemented; the
remaining surface is **UDP datagrams** (§11 / §10.3) and the **`tls` package**
(§10.4).

It complements:

- `specifications/standard_package.md`
- `specifications/architecture.md`
- `specifications/error_codes.md`
- `specifications/linker.md`
- `specifications/plan-linker.md` (the multi-library / framework / versioned-symbol
  linker work that the `tls` backend drives)

## 1. Goal

Implement the still-missing `net`/`tls` surface so the whole of
`standard_package.md` §11 is real, tested, and identical across
`macos-aarch64` and `linux-aarch64` (glibc and musl).

### 1.1 UDP (§10.3)

Types:

- `UdpSocket` — UDP datagram socket (opaque standard `RESOURCE`, thread-sendable).
- `Datagram` — `Datagram[from AS Address, bytes AS List OF Byte]`.
- `DatagramText` — `DatagramText[from AS Address, value AS String]`.

Functions:

- `net::bindUdp(host AS String, port AS Integer) AS UdpSocket`
- `net::receiveFrom(sock AS UdpSocket, maxBytes AS Integer) AS Datagram`
- `net::receiveTextFrom(sock AS UdpSocket, maxBytes AS Integer) AS DatagramText`
- `net::sendTo(sock AS UdpSocket, address AS Address, bytes AS List OF Byte) AS Nothing`
- `net::sendTextTo(sock AS UdpSocket, address AS Address, value AS String) AS Nothing`
- `net::close(resource AS UdpSocket) AS Nothing`
- `net::localAddress(sock AS UdpSocket) AS Address`
- `net::setReadTimeout(sock AS UdpSocket, timeoutMs AS Integer) AS Nothing`
- `net::setWriteTimeout(sock AS UdpSocket, timeoutMs AS Integer) AS Nothing`

### 1.2 TLS (§10.4)

Type:

- `TlsSocket` — opaque standard `RESOURCE`, wraps a connected TCP stream with
  certificate validation and encrypted I/O.

Functions:

- `tls::connect(host AS String, port AS Integer, timeoutMs AS Integer = 0, serverName AS String = "") AS TlsSocket`
- `tls::read(sock AS TlsSocket, maxBytes AS Integer) AS List OF Byte`
- `tls::readText(sock AS TlsSocket, maxBytes AS Integer) AS String`
- `tls::write(sock AS TlsSocket, bytes AS List OF Byte) AS Nothing`
- `tls::writeText(sock AS TlsSocket, value AS String) AS Nothing`
- `tls::close(resource AS TlsSocket) AS Nothing`

`tls::wrap` (upgrade an existing plain `Socket` in place, STARTTLS-style) is
**removed** from the standard package. It cannot be supported on macOS — the
future-proof Network.framework owns its own socket and has no API to secure an
existing fd, and the only fd-capable macOS API (Secure Transport) is deprecated.
The unique workload that needs in-place upgrade (hand-rolled Postgres/MySQL wire
TLS) is not realistic in MFBASIC (those use native client libraries via `LINK`);
implicit-TLS ports (465/993/995/636/990) cover the rest via `tls::connect`.

Secure defaults are mandatory (validation on by default, host trust store,
server-name validation on, TLS < 1.2 disabled, prefer TLS 1.3). Insecure modes,
custom trust stores, and pinning are out of scope (§10.4).

## 2. Current State

TCP and DNS are implemented as native runtime helpers and are tested:

- Front end: `src/builtins/net.rs` defines `net.lookup`, `net.connectTcp`,
  `net.listenTcp`, `net.accept`, `net.poll`, `net.read`, `net.readText`,
  `net.write`, `net.writeText`, `net.close`, `net.localAddress`,
  `net.remoteAddress`, `net.setReadTimeout`, `net.setWriteTimeout`, and the
  `Socket`, `Listener`, `Address` types (`Address` via `builtin_type_fields`).
- Registration: `net` is a built-in import (`src/builtins/mod.rs`
  `is_builtin_import`), and the named-argument table in `call_param_names`
  covers the TCP/DNS calls.
- Runtime helper family: `RuntimeHelper::Net` exists in
  `src/target/shared/runtime.rs`, with `NET_*_SPEC` helper specs (one per call,
  plus `net.connectTcpAddr` for the `Address` overload).
- Codegen: `src/target/shared/code/net.rs` emits each helper as a self-contained
  AArch64 function over libc socket calls (`getaddrinfo`, `socket`, `connect`,
  `bind`, `listen`, `accept`, `recv`/`send`, `poll`, `setsockopt`, …). `Socket`
  and `Listener` reuse the `File` record layout (`fd` at 0, `closed` flag at 8).
- Resources: `Socket` (sendable) and `Listener` (not sendable) are registered in
  `src/builtins/resource.rs` `BUILTIN_RESOURCES`.
- Tests: `tests/func_net_*` cover the TCP/DNS calls.

Not present anywhere in `src/`:

- No UDP: no `bindUdp`/`receiveFrom`/`receiveTextFrom`/`sendTo`/`sendTextTo`,
  no `UdpSocket`/`Datagram`/`DatagramText` types.
- No `tls` anything: `tls`/`TlsSocket` are not built-in imports/types, there is
  no `tls.*` helper family, and no codegen. (The `tls` tokens that appear under
  `src/arch/` and `src/os/macos/link.rs` are thread-local-storage, unrelated to
  transport TLS.)
- `net::poll(List OF Socket)` (§11) is intentionally omitted and stays omitted:
  the ownership model forbids resource handles in collections, so a
  `List OF Socket` value cannot be constructed (documented in `net.rs` and the
  spec). No work here.

The error registry already reserves what this plan needs: `ErrMessageTooLarge`
(`77070007`) for oversized datagrams and `ErrTlsFailed` (`77070008`) for TLS
failures (`specifications/error_codes.md`). No new error codes are required.

## 3. UDP Plan

UDP is a straight extension of the existing native-helper model. It needs **no
new linker capability** — it uses the same libc socket calls already imported
for TCP, plus `recvfrom`/`sendto`.

### 3.1 Front end (`src/builtins/net.rs`)

- Add `UDP_SOCKET_TYPE = "UdpSocket"`, `DATAGRAM_TYPE = "Datagram"`,
  `DATAGRAM_TEXT_TYPE = "DatagramText"`.
- Add call constants `BIND_UDP`, `RECEIVE_FROM`, `RECEIVE_TEXT_FROM`, `SEND_TO`,
  `SEND_TEXT_TO`, and extend `is_net_call`.
- Extend `is_builtin_type` and `builtin_type_fields`:
  - `Datagram` → `[("from", "Address"), ("bytes", "List OF Byte")]`
  - `DatagramText` → `[("from", "Address"), ("value", "String")]`
  - `UdpSocket` is an opaque resource (no fields).
- Extend `resource_close_function` so `UdpSocket` → `net.close`.
- Extend `call_return_type_name`, `resolve_call`, `expected_arguments`,
  `argument_types`, and `arity` for the five new calls and the `UdpSocket`
  overloads of `close`, `localAddress`, `setReadTimeout`, `setWriteTimeout`.
  Note `net.close`/`net.localAddress`/`net.setReadTimeout`/`net.setWriteTimeout`
  become overloaded on `UdpSocket` in addition to `Socket`/`Listener`.

### 3.2 Registration (`src/builtins/mod.rs`)

- Add `net.bindUdp`, `net.receiveFrom`, `net.receiveTextFrom`, `net.sendTo`,
  `net.sendTextTo` to `call_param_names`, and widen the existing `net.close` /
  `net.localAddress` / `net.set*Timeout` entries to accept the `UdpSocket`
  binding name.
- `is_builtin_type` already delegates to `net::is_builtin_type`, so the new
  record/resource types are picked up automatically.

### 3.3 Resource (`src/builtins/resource.rs`)

- Register `UdpSocket` in `BUILTIN_RESOURCES`: `close_function = net.close`,
  `sendable = true` (spec §11: `UdpSocket` is thread-sendable and moves on
  `thread::send`), `close_may_fail = true`, `kind = Builtin`.

### 3.4 Runtime helpers and codegen

- Add `RuntimeHelperSpec`s under `RuntimeHelper::Net`: `net.bindUdp`,
  `net.receiveFrom`, `net.receiveTextFrom`, `net.sendTo`, `net.sendTextTo`,
  plus the `UdpSocket` close/localAddress/timeout variants (reuse the existing
  close/localAddress/timeout helper bodies where the `fd`/`closed` layout is
  identical — `UdpSocket` should share the `File` record layout like `Socket`).
- Implement the bodies in `src/target/shared/code/net.rs`:
  - `bindUdp`: `getaddrinfo` (or numeric host) → `socket(AF_INET, SOCK_DGRAM)` →
    `bind`. `SOCK_DGRAM` is `2`; add a constant beside `SOCK_STREAM`.
  - `receiveFrom`/`receiveTextFrom`: `recvfrom` into a `maxBytes` buffer with a
    `sockaddr_storage` out-param; build an `Address` from the sender, then a
    `Datagram`/`DatagramText` record. **Reject truncation**: if the datagram is
    larger than `maxBytes`, fail with `ErrMessageTooLarge` (`77070007`) rather
    than returning a silently truncated payload (§10.3). On many platforms a
    truncated `recvfrom` is detectable via `MSG_TRUNC`; where it is not, size the
    receive buffer to detect overflow.
  - `sendTo`/`sendTextTo`: resolve `address` to a `sockaddr_in` (reuse the
    `getaddrinfo`/`sin_port`-write path already used by TCP) and `sendto`.
  - `localAddress(UdpSocket)`: `getsockname` (same as the socket path).
  - `setReadTimeout`/`setWriteTimeout(UdpSocket)`: `setsockopt(SO_RCVTIMEO /
    SO_SNDTIMEO)` — reuse the existing socket timeout helper.
- Wire `helper_for_call`, `supported_helper_specs`, and any
  `is_native_direct_call` list, mirroring the existing TCP entries.

### 3.5 Target plans

- macOS `src/target/macos_aarch64/plan.rs` and Linux
  `src/target/linux_aarch64/plan.rs`: add `recvfrom`/`sendto`/`getsockname`
  imports for the new helper symbols if they are not already imported for TCP.
  These live in `libSystem` (macOS) and `libc` (Linux); **no new library** is
  introduced. Both Linux flavors (glibc, musl) get the same symbols.

## 4. TLS Plan

**Status: DONE and verified on Linux (OpenSSL) and macOS (Network.framework).**
`tls` is a **native built-in package** (like `net`). Each platform drives its
system TLS stack through `dlopen`/`dlsym` issued from hand-written runtime
helpers: OpenSSL on Linux, Network.framework on macOS. `tls::wrap` was removed
from the package (see §1.2) so the surface is `connect`/`read`/`readText`/
`write`/`writeText`/`close` on both.

### 4.1 Settled backend decisions

- **Linux = system OpenSSL, native built-in (not `LINK`).** `tls` is a built-in
  with its own `RuntimeHelper::Tls` family and AArch64 codegen in
  `src/target/shared/code/tls.rs`. Each helper `dlopen`s `libssl` (trying
  `libssl.so.3`, then `libssl.so.1.1`) and `dlsym`s the `SSL_*` symbols it needs.
  `dlsym` resolves the library's *default* symbol version, so one binary spans
  **OpenSSL 1.1.1 and 3.x** with no versioned-symbol imports and no `DT_NEEDED`
  on `libssl` — only `dlopen`/`dlsym` are imported (from libc). A `TlsSocket` is a
  32-byte arena record `{ fd, closed, SSL*, SSL_CTX* }`.
- **Why native, not `LINK`.** The `LINK` path was investigated and set aside: the
  opaque-resource model has nowhere to hold per-session buffer state, and `LINK`
  lacks (a) versioned-soname + fallback loading, (b) length-delimited binary
  buffer marshaling for `SSL_read`/`SSL_write`, and (c) access to a connected
  socket fd for `SSL_set_fd`. The native helpers handle all three directly —
  buffer I/O reuses the same patterns as the UDP `receiveFrom`/`sendTo` helpers,
  the fd lives in the record, and the soname fallback is just two `dlopen` calls.
- **Why `dlopen` over static versioned symbols.** The original plan (OpenSSL 3
  only, `@@OPENSSL_3.0.0`, `DT_NEEDED libssl.so.3`) cannot run on Arch ARM, which
  ships only OpenSSL 1.1.1. A single statically-linked soname cannot span `.so.3`
  and `.so.1.1`; `dlopen` with an ordered fallback can.
- **Secure defaults (enforced).** `SSL_CTX_set_default_verify_paths` (host trust
  store), `SSL_set_verify(SSL_VERIFY_PEER)`, `SSL_set1_host` (hostname/cert-name
  validation), `SSL_set_tlsext_host_name` via `SSL_ctrl` (SNI), and minimum
  protocol `TLS1_2_VERSION` via `SSL_ctrl`. `SSL_get_verify_result` is also
  checked. Any handshake/cert/SNI/protocol failure → `ErrTlsFailed` (`77070008`).
- **macOS = Network.framework (`connect`-only).** `tls.connect` builds an
  `nw_connection` (TLS over TCP, `nw_parameters_create_secure_tcp` with the
  default secure configuration) on a dispatch queue, and bridges the async
  state-changed / send / receive callbacks to a synchronous ABI with a
  `dispatch_semaphore`. The three completion handlers are hand-emitted
  Objective-C block literals (`&_NSConcreteStackBlock` + a static descriptor +
  emitted `invoke` thunks) capturing a small arena context `{ sem, signal_fn,
  out_state, out_content, out_error, retain_fn }`; received `dispatch_data` is
  retained inside the block and mapped with `dispatch_data_create_map`.
  Network.framework validates the chain + hostname by default → `ErrTlsFailed` on
  failure. Network.framework owns its socket and cannot wrap an existing fd, which
  is why `tls::wrap` was dropped (§1.2). Everything is resolved via
  `dlopen`/`dlsym` of `Network.framework`; only `dlopen`/`dlsym` are imported.

#### 4.1.1 Verified on all four target machines

Built `linux-aarch64` (glibc + musl) and ran over SSH; built `macos-aarch64` and
ran on the host:

| Machine | libc / OS | TLS backend | handshake | bad-cert → `ErrTlsFailed` |
| --- | --- | --- | --- | --- |
| Arch Linux ARM | glibc, OpenSSL 1.1.1 | OpenSSL (`libssl.so.1.1` fallback) | ✅ | ✅ |
| Kali | glibc, OpenSSL 3.0 | OpenSSL (`libssl.so.3`) | ✅ | ✅ |
| Alpine | musl, OpenSSL 3.5 | OpenSSL (`libssl.so.3`) | ✅ | ✅ |
| macOS | macOS arm64 | Network.framework | ✅ | ✅ |

Handshake proven against `example.com:443` (real cert + SNI + encrypted
read/write, both byte and text). Validation-on-by-default proven against
`self-signed.badssl.com`,
`wrong.host.badssl.com`, and `expired.badssl.com` — all three rejected with
`77070008` on every machine, while `example.com` succeeded.

### 4.2 Package surface — `tls` is its own package qualifier

`tls::*` is a distinct package from `net::*`, even though it is documented inside
§11. The Linux backend uses option **(a)** — a native-helper package mirroring
`net`:

- **(a) Native-helper package, mirroring `net` (implemented).** `src/builtins/
  tls.rs` (front-end signatures, `TlsSocket` type, default args, consume
  semantics), registered in `mod.rs` / `resource.rs` / `resolver.rs`; a
  `RuntimeHelper::Tls` family; AArch64 codegen in `src/target/shared/code/tls.rs`
  that `dlopen`/`dlsym`s OpenSSL directly. `TlsSocket` is a first-class built-in
  resource.
- (b) MFBASIC-source package over a `LINK` block. Set aside: the opaque-resource
  model cannot hold per-session buffer state, and `LINK` lacks versioned-soname
  fallback, binary-buffer marshaling, and socket-fd access (see §4.1).

### 4.3 Front end (`src/builtins/tls.rs`)

- `TLS_SOCKET_TYPE = "TlsSocket"`; opaque resource, no user fields.
- Call constants and resolution for `tls.connect`, `tls.read`, `tls.readText`,
  `tls.write`, `tls.writeText`, `tls.close`, with the default arguments from
  §10.4 (`timeoutMs = 0`, `serverName = ""`).
- `tls.close` **consumes** the `TlsSocket` it closes.

### 4.4 Registration and resource

- `mod.rs`: add `tls` to `is_builtin_import`; route `tls::is_builtin_type` and
  `tls::call_return_type_name`; add `tls.*` entries to `call_param_names`.
- `resource.rs`: register `TlsSocket` in `BUILTIN_RESOURCES`:
  `close_function = tls.close`. **`sendable = false` for v1** — sending a TLS
  session across threads adds bridge/state-ownership complexity with no spec
  requirement (§9 says each handle type opts into sendability explicitly; do not
  opt in here yet).

### 4.5 Runtime helpers and codegen

- New `RuntimeHelper::Tls` variant in `src/target/shared/runtime.rs`; add it to
  `name()` and confirm it round-trips through `src/binary_repr.rs` (the
  `runtime_helpers` list a package records). Mirror the `Net` variant everywhere
  `Net` is matched.
- Helper specs for `tls.connect`, `tls.read`, `tls.readText`,
  `tls.write`, `tls.writeText`, `tls.close`.
- Codegen is **target-specific** in a way TCP/UDP are not, because the backend
  libraries differ:
  - **Linux (`src/target/shared/code/tls.rs` + Linux plan):** drive OpenSSL 3 —
    `SSL_CTX_new(TLS_client_method())`, default verify paths /
    `SSL_CTX_set_default_verify_paths`, `SSL_set1_host` (or `X509_VERIFY_PARAM`)
    for SNI + name validation, `SSL_set_tlsext_host_name`, minimum version
    `TLS1_2_VERSION`, `SSL_new`/`SSL_set_fd`/`SSL_connect`, then
    `SSL_read`/`SSL_write`/`SSL_shutdown`/`SSL_free`. `TlsSocket` carries the
    underlying `fd`, the `SSL*`, and a `closed` flag; choose a fixed record
    layout and document it beside the `File` layout note in `code/net.rs`.
  - **macOS (Network.framework):** create an `nw_connection` from a host/port
    endpoint + `nw_parameters_create_secure_tcp` (default secure configuration),
    start it on a dispatch queue, and bridge the async state-changed / send /
    receive completion handlers to the synchronous MFBASIC ABI (hand-emitted
    block literals + `dispatch_semaphore`). This bridge is the main macOS-specific
    cost. Default parameters enforce chain + hostname validation. (See §4.1 for
    the implemented details.)
- Because a package decodes back to IR and lowers through the same
  `IR → NIR → native` path (`architecture.md` §11), the *front-end* signatures
  and IR are target-independent; only the emitted helper bodies and the platform
  import lists differ per target. Keep the divergence inside `code/tls.rs` +
  the two `plan.rs` files.

### 4.6 Target plans

- **Linux** `linux_aarch64/plan.rs` (implemented): the `tls.*` helpers import
  only `dlopen`/`dlsym` (from libc) plus `__errno_location`; `tls.connect` also
  imports `getaddrinfo`/`freeaddrinfo`/`socket`/`connect`/`close` for its own TCP
  setup. There is **no `DT_NEEDED` on `libssl`** — OpenSSL is loaded at runtime
  via `dlopen` with the `libssl.so.3` → `libssl.so.1.1` fallback, which is why one
  binary runs against either OpenSSL series. Identical for glibc and musl.
- **macOS** `macos_aarch64/plan.rs` (implemented): the `tls.*` helpers import only
  `_dlopen`/`_dlsym`/`___error` from `libSystem`. Network.framework, libdispatch,
  and the block runtime (`_NSConcreteStackBlock`) are all resolved at runtime via
  `dlopen` of `Network.framework`; there is no framework link dependency.
- Keep all library divergence in the target `plan.rs` files; the shared front end
  and IR stay platform-neutral.

## 5. Error Behavior

- UDP oversized datagram → `ErrMessageTooLarge` (`77070007`); never silently
  truncate (§10.3).
- TLS handshake / certificate / SNI / protocol failure → `ErrTlsFailed`
  (`77070008`).
- Reuse the existing network error mapping for the shared transport failures
  (`ErrAddressInvalid` `77070001`, `ErrAddressNotFound` `77070002`,
  `ErrNetworkFailed` `77070003`, `ErrConnectionClosed` `77070004`,
  `ErrReadTimeout` `77070005`, `ErrWriteTimeout` `77070006`,
  `ErrInvalidArgument` `77050002`, `ErrResourceClosed` `77030004`).
- Negative timeouts are invalid → `ErrInvalidArgument` (§11 timeout table).
- No new error codes are introduced by this plan.

## 6. Validation Plan

### 6.1 Function tests

Add mandatory valid and invalid directories per new function, matching the
existing `tests/func_net_*` convention:

- UDP: `func_net_bindUdp_*`, `func_net_receiveFrom_*`,
  `func_net_receiveTextFrom_*`, `func_net_sendTo_*`, `func_net_sendTextTo_*`,
  and `UdpSocket` cases for `func_net_close_*` / `func_net_localAddress_*` /
  `func_net_setReadTimeout_*` / `func_net_setWriteTimeout_*`.
- TLS: `func_tls_connect_*`, `func_tls_read_*`, `func_tls_readText_*`,
  `func_tls_write_*`, `func_tls_writeText_*`, `func_tls_close_*`.

Coverage should include wrong arity, wrong argument types, negative/invalid
timeouts, closed-handle reuse, and the datagram-too-large path.

### 6.2 Runtime proofs

These are runtime features; compilation passing is not enough.

- UDP: a loopback send/receive round trip (bind two UDP sockets, exchange a
  datagram, assert sender `Address` and bytes; assert `ErrMessageTooLarge` when
  `maxBytes` is below the datagram size).
- TLS: a real handshake against a known-good endpoint in a gated/networked test
  (or a local test server), proving certificate + SNI validation is on by
  default and that a bad name fails with `ErrTlsFailed`.
- Prove identical observable behavior on `macos-aarch64`, `linux-aarch64`
  glibc, and `linux-aarch64` musl, and prove the same results when the code is
  imported as an MFP package vs. compiled directly.

### 6.3 Acceptance

- Run `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- Not complete until acceptance passes and both UDP and TLS are demonstrated
  end-to-end on all current target flavors.

## 7. Recommended Sequence

1. **UDP first** — it is self-contained, needs no linker work, and finishes a
   large, low-risk slice of §11.
   1. `net.rs` front end + `UdpSocket`/`Datagram`/`DatagramText` types.
   2. `mod.rs` registration; `resource.rs` `UdpSocket` entry.
   3. Helper specs + `code/net.rs` bodies; `plan.rs` `recvfrom`/`sendto`/
      `getsockname` imports (both targets, both Linux flavors).
   4. Tests + runtime loopback proofs; acceptance.
2. ~~Confirm `plan-linker.md` linker capability~~ — **not needed.** The TLS
   backend loads its system TLS stack with `dlopen`/`dlsym` at runtime, so it does
   not depend on the static multi-library / framework / versioned-symbol linker.
3. **TLS** (done) — `src/builtins/tls.rs`, `mod.rs`/`resource.rs` registration,
   `RuntimeHelper::Tls`, `code/tls.rs` bodies (OpenSSL on Linux via
   `libssl.so.3`→`libssl.so.1.1` `dlopen` fallback, Network.framework on macOS),
   and the two `plan.rs` `dlopen`/`dlsym` import sets.
4. Tests + runtime handshake proofs; acceptance across all target flavors (done).

## 8. Non-Goals For This Plan

- `net::poll(List OF Socket)` — permanently omitted (resource-in-collection ban).
- Unix-domain sockets and detailed DNS record inspection (§11: extension-package
  territory).
- TLS insecure modes, custom trust stores, certificate pinning, and TLS server
  (listener) sockets (§10.4: explicit extension APIs, not the minimal core).
- Making `TlsSocket` thread-sendable (deferred; `Socket`/`UdpSocket` remain
  sendable as the spec requires).
- `tls::wrap` (removed from the package surface, §1.2).

## 9. Bottom Line

**Done.** UDP landed as a mechanical extension of the TCP native-helper
machinery with no linker changes. TLS is a native built-in that loads its system
TLS stack via `dlopen`/`dlsym` (no linker capability needed), and is the only
part of §11 with per-target backend divergence (OpenSSL on Linux, with a
`libssl.so.3`→`libssl.so.1.1` fallback; Network.framework on macOS) — kept
isolated to `code/tls.rs` and the two `plan.rs` files so the front end and IR
stay platform-neutral. Verified end-to-end on macOS plus `linux-aarch64` glibc
(Arch, Kali) and musl (Alpine).
