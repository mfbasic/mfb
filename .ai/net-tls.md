# Networking, TLS & repository-client invariants

Invariants for `tls::connect` timeout semantics, TLS read readiness, and the repository client's transport-security enforcement.

## tls::connect timeoutMs=0 is one immediate attempt

`tls::connect(host, port, timeoutMs, serverName)` follows the language timeout convention: **`timeoutMs = 0` is one immediate, non-blocking attempt** — it succeeds only if connect+handshake complete synchronously, else raises `ErrTimeout` (`7-705-0008`, "Operation did not complete before its deadline") **instantly** (macOS uses `DISPATCH_TIME_NOW`). The *omitted* 3-arg form is the unbounded one, NOT `0`. A negative value raises `ErrInvalidArgument`.

So a real TLS 1.3 handshake to a remote host can essentially never pass with `timeoutMs=0` — it fails in ~4ms with an error that *looks* like a slow-network deadline but isn't (the network can be perfectly healthy; `openssl s_client` to the same host succeeds). Diagnostic tell: a "deadline" error that returns in single-digit ms is a `0`-timeout immediate attempt, not a network wait.

A live-network fixture must therefore pass a positive `timeoutMs`. Example: `tests/rt-behavior/tls/tls-connect-google-rt` had `tls::connect("8.8.8.8", 443, 0, "dns.google")` while its golden expected `connected=TRUE` — internally contradictory. The fix is the fixture: use a positive `timeoutMs` (e.g. 5000) so the live connect reliably completes when the network is up. Such a failure is easy to mislabel as a "network flake" when it is actually a `timeoutMs=0` fixture bug. The acceptance golden harness's `test-gate.sh` skip hook covers the genuinely-offline case.

## TLS readiness must check the decrypted-buffer, not just the fd

A TLS socket's "is a read ready?" is **not** an fd `poll(2)`. One TLS record decrypts to many application bytes; a single `SSL_read`/Network.framework receive drains a record and **buffers the remainder**, so the TLS layer can hold already-decrypted app bytes while the raw fd is idle. An fd-only poll then reports "not ready" while a byte is available — a correctness bug. Readiness = `(TLS-buffered app bytes > 0) OR (raw layer readable)`, and the buffered half is backend-specific:

- **openssl** (`src/target/shared/code/tls/openssl.rs`): `SSL_pending(ssl)` for buffered count, else `poll(fd, POLLIN)`.
- **schannel** (Windows, `tls/schannel_io.rs` / `schannel_read_close.rs`): a decrypted-record carry-over buffer, else `WSAPoll(POLLRDNORM)` (`schannel_impl.rs` `WSAPOLLFD`).
- **macOS** (`tls/macos/`): **Network.framework — there is NO raw fd.** Decrypted data lands in a single-producer/single-consumer ring of pending retained buffers (`macos/mod.rs:106`), drained on the owning thread; readiness = ring non-empty OR terminal state, and any bounded wait uses the read path's `dispatch_semaphore` (which already has a per-read release/leak hazard, `macos/mod.rs:418` — a poll waiting on it must not steal the read's signal or leak a semaphore).

`tls` now has `poll` (plan-76-B/C) and, since plan-110-D, `localAddress`/`remoteAddress` and `setReadTimeout`/`setWriteTimeout`. Related: the resources ownership model, and `tcp::poll` fd scaffolding.

## A TLS read/write deadline is a socket option on two platforms and a ctx field on the third

`tls::setReadTimeout`/`setWriteTimeout` reuse `net`'s `setsockopt(SO_RCVTIMEO/SO_SNDTIMEO)` emitter verbatim on Linux and Windows — their TLS record keeps the descriptor in the canonical handle slot, so the option lands on the same fd `tcp` would use. **macOS cannot**: Network.framework owns the socket. The deadline is stored on the connection ctx (`CTX_RTO`/`CTX_WTO`, sentinel = unbounded) and applied by `emit_wait_bounded` at the `dispatch_semaphore_wait` that actually blocks.

Three traps in that, all load-bearing:

- **A timed-out read must leave its receive outstanding.** `nw_connection_receive` has no cancel, so its completion block will still fire. The read path therefore posts the *poll-style* block (`RECV_POLL_INVOKE` → `CTX_PCONTENT`/`CTX_PSEM`) and sets `CTX_ARMED`; the timeout exit clears nothing and releases nothing, so the next read drains that same receive instead of posting a second one. Clearing `CTX_ARMED` there strands the bytes and leaks the content object.
- **A timed-out write must be drained before the next one recycles CTX_SEM.** `tls::write` calls `emit_fresh_sem`, and replacing the semaphore under an in-flight send is the bug-52/55 hazard. `CTX_WARMED` marks the outstanding send; write drains it (bounded) *before* the fresh-sem call.
- **`tls::read` must NOT call `emit_fresh_sem`.** It waits on `CTX_PSEM` now; recycling `CTX_SEM` would break a concurrent-in-the-same-thread outstanding send.

## The deadline error code is `ErrTimeout` on every backend — and each one had to be taught it

The convention says a deadline raises `ErrTimeout` (77050008). Left alone, each backend reported its own transport error instead, and the three disagreed for the identical event:

| backend | what expiry looks like | reported before plan-110-D |
|---|---|---|
| macOS | `dispatch_semaphore_wait` returns non-zero | ErrTimeout (correct) |
| Linux | `SSL_read` <= 0, `SSL_get_error` = `SSL_ERROR_WANT_READ`(2)/`WANT_WRITE`(3) | ErrTlsFailed |
| Windows | `recv` = SOCKET_ERROR, `WSAGetLastError` = `WSAETIMEDOUT` (10060, **not** EWOULDBLOCK) | ErrNetworkFailed |

If you add a bounded operation to any backend, classify the expiry explicitly — the natural error path will otherwise swallow it as a transport failure and a caller cannot tell a slow peer from a broken session.

## A macOS TLS `Listener` has no descriptor, and its whole address surface is one port

The pattern above repeats for endpoint queries, but harder. `tls::localAddress`/`remoteAddress` over a **`Socket`** reuse `net`'s `getsockname`/`getpeername` emitter on Linux and Windows, and macOS substitutes `nw_connection_copy_current_path` → `nw_path_copy_effective_{local,remote}_endpoint` → `nw_endpoint_get_address`, which still yields a `sockaddr` and so still feeds the shared `Address` builder.

A **`Listener`** has no such escape. Its handle slot holds an `nw_listener`, and Network.framework's entire listener API (checked against `Network.framework/Headers/listener.h`) exposes exactly one address accessor: `nw_listener_get_port` — a `uint16_t` in **host** byte order, and no address at all. There is no `nw_listener_copy_parameters` and no `nw_listener_copy_endpoint`, so the bound address cannot be recovered from the listener after the fact.

So `tls::listen` parks the host C string it already built for `nw_endpoint_create_host` in a listener-record tail slot (`REC_LHOST` = 48), and `tls::localAddress(listener)` pairs it with `nw_listener_get_port`. Consequences to know before touching this:

- **The stored pointer is borrowed, never freed.** It is either an arena copy of the caller's host or the static `_mfb_tls_anyhost` (`"0.0.0.0"`), both valid for the life of the process. Do not add a release for it, and do not switch it to a retained `nw_endpoint` — that would need a matching `nw_release` in the listener close path for no gain.
- **macOS reports the host as *bound*, not as *resolved*.** `tls::listen("localhost", 0, …)` reports `localhost` where a `getsockname` read-back reports `127.0.0.1`. Numeric hosts — the port-0 case that matters — agree everywhere. This is documented on the member; there is no API that would close it.
- **`nw_listener_get_port` is listed with the CLIENT symbols on purpose.** The `localAddress` overload split resolves at emission, so the code layer force-emits the listener body wherever `tls.localAddress` appears — including in a client-only module, which would otherwise relocate against a name the server-gated symbol table never wrote. Gating the *synthesis* does not close it either: a module can take a `Listener` parameter without ever calling `listen`.
- **The port needs a 16-bit store.** `nw_listener_get_port` returns `uint16_t`; the C return's upper bits are undefined, so it goes into a zeroed slot via `store_u16`. Unlike the `sockaddr` path, no byte-swap: it is already host order.

## There is no `tls::wrap`, and the reason is macOS-specific

Upgrading an established `tcp::Socket` in place needs to adopt its fd. On macOS nothing supported can: Network.framework fixes TLS in `nw_parameters` at creation and cannot graft it onto a live connection; `nw_connection_create_with_connected_socket` is exported but declared in no SDK header and fails `ENETDOWN` for every parameter shape; Secure Transport can adopt an fd but is deprecated and rejects `kTLSProtocol13` (`errSSLIllegalParam`), capping at TLS 1.2. The system LibreSSL (`/usr/lib/libssl.48.dylib`) *can* do it at TLS 1.3 — measured — but ships no headers and the unversioned path deliberately aborts, so it is unsupported. Shipping `wrap` on Linux and Windows alone would let a program compile for five targets and fail at runtime on one, so the member exists nowhere (plan-110-D §C9). Do not reintroduce it on two platforms.

## Repository client transport security is per-URL, not per-hop

In `repository/src/client.rs`, `ensure_transport_security(repo_url)` validates ONLY the initial URL (https, or http-loopback for local dev). It is called at each network entry point (`get_json`/`post_json`/`fetch_blob`/…) but does NOT follow redirects, so a 302 target is never re-checked by it.

The one shared `reqwest::blocking::Client` is built once in `http_client()` (a `OnceLock`) — that is the ONLY place `connect_timeout` AND the redirect policy can be set. Per-hop transport enforcement therefore lives in `redirect_policy()` / `ensure_redirect_target()`, not in `ensure_transport_security`. The redirect guard is https-only and blocks private/loopback/link-local/CGNAT/unspecified IP literals (incl. IPv4-mapped IPv6) — otherwise a hostile registry 302 drives blind SSRF (169.254.169.254, 127.0.0.1, RFC-1918) or an https→http downgrade leak. Blob bytes stay SHA-256 checked and control-plane bodies signature-checked regardless; the redirect guard only closes the transport-level leak.

Takeaway: if you add a new registry route or loosen networking, remember the initial-URL check and the redirect check are SEPARATE — enforce both. `reqwest::redirect::Policy::custom` closures track depth via `attempt.previous().len()` and reject a hop with `attempt.error(String)`.
