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
- `tls::wrap(sock AS Socket, serverName AS String, timeoutMs AS Integer = 0) AS TlsSocket`
- `tls::read(sock AS TlsSocket, maxBytes AS Integer) AS List OF Byte`
- `tls::readText(sock AS TlsSocket, maxBytes AS Integer) AS String`
- `tls::write(sock AS TlsSocket, bytes AS List OF Byte) AS Nothing`
- `tls::writeText(sock AS TlsSocket, value AS String) AS Nothing`
- `tls::close(resource AS TlsSocket) AS Nothing`

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

TLS is the hard part. Unlike UDP, it forces the linker to grow: multiple
libraries, macOS framework loading, and Linux versioned-symbol imports. The
linker work is owned by `specifications/plan-linker.md`; **this plan depends on
that linker capability landing first** (or in lockstep).

### 4.1 Settled backend decisions

Per `plan-linker.md` (do not relitigate here):

- **Linux = OpenSSL 3 only**: `libssl.so.3` + `libcrypto.so.3`, versioned
  symbols `@@OPENSSL_3.0.0`. No `libssl.so.1.1` fallback. Same for glibc and
  musl.
- **macOS = Network.framework** (async/dispatch/block based). Chosen over the
  deprecated Secure Transport / Security.framework. The async→sync bridge is a
  `tls` codegen cost, not a linker cost.
- `tls` is the *driver* for multi-library + framework + versioned-symbol linking;
  data globals (`GLOB_DAT`) and load-time initializers are app-mode concerns and
  are **not** required by `tls`.

### 4.2 Package surface — `tls` is its own package qualifier

`tls::*` is a distinct package from `net::*`, even though it is documented inside
§11. Two implementation options; recommend **(a)**:

- **(a) Native-helper package, mirroring `net`.** Add `src/builtins/tls.rs`
  (front-end signatures, `TlsSocket` type, `resource_close_function`), register
  `tls` in `mod.rs` `is_builtin_import` / `is_builtin_type` / `call_param_names`,
  add a `RuntimeHelper::Tls` family, and emit bodies from a new
  `src/target/shared/code/tls.rs`. This matches how `net` is built and keeps
  `TlsSocket` a first-class built-in resource.
- (b) MFBASIC-source package over a small native primitive set (the `json`
  precedent). Rejected: TLS state, buffering, and the macOS async→sync bridge are
  awkward to express in MFBASIC and would still need native primitives, so it
  buys little.

### 4.3 Front end (`src/builtins/tls.rs`)

- `TLS_SOCKET_TYPE = "TlsSocket"`; opaque resource, no user fields.
- Call constants and resolution for `tls.connect`, `tls.wrap`, `tls.read`,
  `tls.readText`, `tls.write`, `tls.writeText`, `tls.close`, with the default
  arguments from §10.4 (`timeoutMs = 0`, `serverName = ""`).
- `tls.wrap` **consumes** its `Socket` argument (the plain socket must not be
  used afterward). Model this the same way the ownership system already handles a
  resource passed to a consuming op — `tls.wrap` takes ownership of the `Socket`
  and returns a `TlsSocket`; the source `Socket` binding is moved out and not
  dropped by the caller (align with how `thread::transfer`-style moves are
  represented).

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
- Helper specs for `tls.connect`, `tls.wrap`, `tls.read`, `tls.readText`,
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
  - **macOS (Network.framework):** create an `nw_connection` over the existing
    socket/endpoint with a TLS-enabled `nw_parameters`, start it on a dispatch
    queue, and bridge the async send/receive completion handlers to the
    synchronous MFBASIC ABI (block + dispatch semaphore). This bridge is the main
    macOS-specific cost. Default parameters already enforce validation; set the
    minimum TLS version and SNI via `sec_protocol_options`.
- Because a package decodes back to IR and lowers through the same
  `IR → NIR → native` path (`architecture.md` §11), the *front-end* signatures
  and IR are target-independent; only the emitted helper bodies and the platform
  import lists differ per target. Keep the divergence inside `code/tls.rs` +
  the two `plan.rs` files.

### 4.6 Target plans (the linker-facing work)

- **Linux** `linux_aarch64/plan.rs`: add `libssl.so.3` and `libcrypto.so.3`
  imports for the TLS helper symbols, with versioned-symbol imports
  (`@@OPENSSL_3.0.0`). This is the first built-in package needing a non-`libc`
  shared library and versioned symbols — it must ride on the `plan-linker.md`
  capability. Same for glibc and musl flavors.
- **macOS** `macos_aarch64/plan.rs`: add Network.framework (and any required
  `libdispatch`/`libobjc`/Security pieces) imports for the TLS helper symbols —
  the first built-in package needing framework loading.
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
- TLS: `func_tls_connect_*`, `func_tls_wrap_*`, `func_tls_read_*`,
  `func_tls_readText_*`, `func_tls_write_*`, `func_tls_writeText_*`,
  `func_tls_close_*`.

Coverage should include wrong arity, wrong argument types, negative/invalid
timeouts, closed-handle reuse, `tls.wrap` source-socket after-move rejection,
and the datagram-too-large path.

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
2. **Confirm `plan-linker.md` linker capability** (multi-library, macOS
   framework loading, Linux versioned symbols) is available; TLS blocks on it.
3. **TLS** — `src/builtins/tls.rs`, `mod.rs`/`resource.rs` registration,
   `RuntimeHelper::Tls` (+ `binary_repr.rs` round-trip), `code/tls.rs` bodies
   (OpenSSL 3 on Linux, Network.framework on macOS), and the two `plan.rs`
   library/framework import sets.
4. Tests + runtime handshake proofs; acceptance across all target flavors.

## 8. Non-Goals For This Plan

- `net::poll(List OF Socket)` — permanently omitted (resource-in-collection ban).
- Unix-domain sockets and detailed DNS record inspection (§11: extension-package
  territory).
- TLS insecure modes, custom trust stores, certificate pinning, and TLS server
  (listener) sockets (§10.4: explicit extension APIs, not the minimal core).
- Making `TlsSocket` thread-sendable (deferred; `Socket`/`UdpSocket` remain
  sendable as the spec requires).
- `libssl.so.1.1` fallback on Linux (OpenSSL 3 only, settled).

## 9. Bottom Line

UDP is a mechanical extension of the existing TCP native-helper machinery and
should land first with no linker changes. TLS is gated on the
`plan-linker.md` multi-library/framework/versioned-symbol linker and is the only
part of §11 that introduces per-target backend divergence (OpenSSL 3 on Linux,
Network.framework on macOS) — kept isolated to `code/tls.rs` and the two
`plan.rs` files so the front end and IR stay platform-neutral.
