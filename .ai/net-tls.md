# Networking, TLS & repository-client invariants

Invariants for `tls::connect` timeout semantics, TLS read readiness, and the repository client's transport-security enforcement.

## tls::connect timeoutMs=0 is one immediate attempt

`tls::connect(host, port, timeoutMs, serverName)` follows the language timeout convention: **`timeoutMs = 0` is one immediate, non-blocking attempt** — it succeeds only if connect+handshake complete synchronously, else raises `ErrTimeout` (`7-705-0008`, "Operation did not complete before its deadline") **instantly** (macOS uses `DISPATCH_TIME_NOW`). The *omitted* 3-arg form is the unbounded one, NOT `0`. A negative value raises `ErrInvalidArgument`.

So a real TLS 1.3 handshake to a remote host can essentially never pass with `timeoutMs=0` — it fails in ~4ms with an error that *looks* like a slow-network deadline but isn't (the network can be perfectly healthy; `openssl s_client` to the same host succeeds). Diagnostic tell: a "deadline" error that returns in single-digit ms is a `0`-timeout immediate attempt, not a network wait.

A live-network fixture must therefore pass a positive `timeoutMs`. Example: `tests/rt-behavior/tls/tls-connect-google-rt` had `tls::connect("8.8.8.8", 443, 0, "dns.google")` while its golden expected `connected=TRUE` — internally contradictory. The fix is the fixture: use a positive `timeoutMs` (e.g. 5000) so the live connect reliably completes when the network is up. Such a failure is easy to mislabel as a "network flake" when it is actually a `timeoutMs=0` fixture bug. The acceptance golden harness's `test-gate.sh` skip hook covers the genuinely-offline case.

## `tcp` and `tls` are drop-in mirrors — including on close and on `listen`

Two answers used to differ, both deliberately, both documented on each other's
pages, and neither reconciled (bug-525):

- **A second close.** `tls::close` set the closed flag once and then returned OK
  forever after; `tcp`/`udp`/`fs` refuse with `ErrResourceClosed`. The language
  spec had already settled it — `mfb spec language resource-management` §15: "an
  already-closed record is flagged, and a second close is a defined no-op
  reported as `ErrResourceClosed` rather than an operation on a dead handle" — so
  `tls` (and `audio`, which had the same shape) was the non-conforming side, not
  a second valid policy. Every close emitter now ends its already-closed arm in
  `emit_fail(.., "ErrResourceClosed", ..)`. Three of the ten emitters had the
  SUCCESS path **falling through** into the `already` label and sharing its OK
  tag (`gen_schannel_read_close`, both audio macOS closes, ALSA, WASAPI), so each
  needed an `abi::branch(&done)` inserted before the label — without it the fix
  makes the *first* close fail.
- **`listen`'s backlog.** `tcp` padded `128` in the code layer, `tls` filled `0`
  from its descriptor. There is one constant now,
  `builtins::net::DEFAULT_LISTEN_BACKLOG`, read by both.

A raising close is safe against the drop path on every resource here because all
of them declare `close_may_fail: true` and §15 discards a drop-close failure —
`fs` has worked exactly this way since it was written. Pins:
`codegen::resource::tests::every_builtin_close_refuses_an_already_closed_handle`
lowers all ten emitters and asserts the `ErrResourceClosed` relocation (the only
instrument that reaches Schannel and WASAPI from a Mac), and
`tests/rt_double_close_is_refused.rs` measures it at runtime for
`fs`/`tcp`/`udp`/`tls`. `audio::close` needed `ErrResourceClosed` added to
`data_objects.rs`'s per-package gate; a scope-exit close is emitted by cleanup
rather than as a NIR call, which is the same trap bug-249 records for `tls`.

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

## A departed peer is `ErrConnectionClosed` on every backend — and each one had to be taught it

Exactly the shape of the deadline row above, one direction over. `tcp` and `tls`
are documented drop-in mirrors and both `read` calls already agreed at end of
stream (bug-465); the WRITE direction named one event three ways, and only the
OpenSSL row matched what `mfb man tcp write` and `mfb man tls write` promise:

| backend | `tls::write` after the peer goes away | measured |
|---|---|---|
| Linux, OpenSSL | `ErrConnectionClosed` (correct since bug-467) | box 2228 |
| macOS, Network.framework | `ErrTlsFailed` — "TLS handshake, certificate validation, SNI validation, or protocol operation failed", for a session whose handshake succeeded | host |
| Windows, Schannel | `ErrNetworkFailed` — "Network operation failed **before a connection was established**", for a connection that was established and used | box 2230 |

Windows is `WSAGetLastError` after `send`: 10054/10053/10058 are the peer,
**10060 `WSAETIMEDOUT` is the `SO_SNDTIMEO` deadline** (the write side had never
learned the `ErrTimeout` rule the read side got in plan-110-D), anything else
stays `ErrNetworkFailed`. macOS is the paragraph below.

## A Network.framework completion block's `nw_error` does NOT outlive the block

The `nw_error_t` a `nw_connection_send`/state-changed handler is handed is
borrowed for the block's duration; Network.framework releases it on return. The
ctx slot `CTX_ERROR` therefore holds a **dangling pointer** by the time the
helper's `dispatch_semaphore_wait` returns, and it has always been read as a
non-null test only — never dereferenced — for that reason.

Measured, because it is not obvious and the failure is intermittent: a build that
called `nw_error_get_error_domain(CTX_ERROR)` from `tls::write` after the wait
SIGSEGV'd in **2 of 6** runs, **3 of 5** under `MallocScribble=1`, and the runs
that survived disagreed with each other about the domain (POSIX 32, POSIX 54,
non-POSIX) for one identical scenario. A design that reads the error object from
the helper is not a slow leak or a rare edge — it is a crash in a server's write
path, driven by a remote peer.

So anything you want to know about an `nw_error` must be asked **inside the block
trampoline**, on the dispatch queue. bug-483 parks
`nw_error_get_error_domain` in the ctx (`CTX_EDOMFN`) at connection-ctx setup and
both `SEND_INVOKE` and `STATE_INVOKE` record the domain into `CTX_EDOM` while the
object is alive; `tls::write` then classifies from that integer.

Three things that design has to get right, all of them load-bearing:

- **`CTX_EDOM` is sticky.** `emit_fresh_sem` clears `CTX_ERROR` before every
  operation and must NOT clear the domain: the terminal-state guard on a *later*
  write is a reader, and by then no send has run.
- **Both trampolines must classify, not just the send.** Measured against a
  departed peer, only the FIRST failing write takes the send-completion error
  path — every write after it hits the `CTX_STATE >= 4` guard, because
  Network.framework has moved the connection to `failed` (4; never `cancelled`,
  5). And the state handler can get there *before* the program's first write ever
  posts a send, which is a 2-in-5 flake if only the send classifies.
- **`STATE_INVOKE` is shared by connection AND listener contexts, so every slot
  it writes must mean the same thing in both.** The listener ctx's ring occupies
  64..192, so the new pair sits at 192/200 with `LCTX_SIZE` grown to match. At
  128/136 the state handler would have written ring entries 8 and 9 — and read
  one back as a function pointer to call.

The write-time guard is NOT reclassified wholesale (a handshake failure reaches
`failed` too, in general): it reports `ErrConnectionClosed` only when a
`nw_error_domain_posix` error was actually recorded on that connection, and
`ErrTlsFailed` otherwise. In practice `tls::connect`/`tls::accept` both wait for
`NW_STATE_READY` before returning a `Socket`, so a certificate failure is raised
there and never reaches `tls::write` at all.

## A macOS TLS `Listener` has no descriptor, and its whole address surface is one port

The pattern above repeats for endpoint queries, but harder. `tls::localAddress`/`remoteAddress` over a **`Socket`** reuse `net`'s `getsockname`/`getpeername` emitter on Linux and Windows, and macOS substitutes `nw_connection_copy_current_path` → `nw_path_copy_effective_{local,remote}_endpoint` → `nw_endpoint_get_address`, which still yields a `sockaddr` and so still feeds the shared `Address` builder.

A **`Listener`** has no such escape. Its handle slot holds an `nw_listener`, and Network.framework's entire listener API (checked against `Network.framework/Headers/listener.h`) exposes exactly one address accessor: `nw_listener_get_port` — a `uint16_t` in **host** byte order, and no address at all. There is no `nw_listener_copy_parameters` and no `nw_listener_copy_endpoint`, so the bound address cannot be recovered from the listener after the fact.

So `tls::listen` parks the host C string it already built for `nw_endpoint_create_host` in a listener-record tail slot (`REC_LHOST` = 48), and `tls::localAddress(listener)` pairs it with `nw_listener_get_port`. Consequences to know before touching this:

- **The stored pointer is borrowed, never freed.** It is either an arena copy of the caller's host or the static `_mfb_tls_anyhost` (`"0.0.0.0"`), both valid for the life of the process. Do not add a release for it, and do not switch it to a retained `nw_endpoint` — that would need a matching `nw_release` in the listener close path for no gain.
- **macOS reports the host as *bound*, not as *resolved*.** `tls::listen("localhost", 0, …)` reports `localhost` where a `getsockname` read-back reports `127.0.0.1`. Numeric hosts — the port-0 case that matters — agree everywhere. This is documented on the member; there is no API that would close it.
- **`nw_listener_get_port` is listed with the CLIENT symbols on purpose.** The `localAddress` overload split resolves at emission, so the code layer force-emits the listener body wherever `tls.localAddress` appears — including in a client-only module, which would otherwise relocate against a name the server-gated symbol table never wrote. Gating the *synthesis* does not close it either: a module can take a `Listener` parameter without ever calling `listen`.
- **The port needs a 16-bit store.** `nw_listener_get_port` returns `uint16_t`; the C return's upper bits are undefined, so it goes into a zeroed slot via `store_u16`. Unlike the `sockaddr` path, no byte-swap: it is already host order.

## Relaxing trust must classify the failure, never disable verification

`tls::connect`'s `allowSelfSigned` (bug-477) accepts a chain that fails *only*
because its root is untrusted. On every backend the shortest way to do that also
silently disables the hostname and expiry checks, so each one **keeps
verification on and classifies the failure** instead.

**bug-177's audit finding is amended, not invalidated.** That audit certified "no
verification bypass exists" on either backend. That remains true of the default
path — omitting the argument is byte-for-byte the old handshake. What exists now
is one opt-in, default-`FALSE`, call-site-visible relaxation of the *trust anchor
only*, reported by `mfb audit` as `AUDIT-TLS-RELAXED-TRUST`.

**OpenSSL: `SSL_VERIFY_NONE` is a trap, measured.** With a NULL callback the
store's default `verify_cb` returns `ok` (0) at the first error, so
`X509_verify_cert` stops inside `build_chain` and `check_id` — the hostname
check — never runs. A self-signed certificate then reports code 18 *whether or
not the name matches*, so accepting {0,18,19,20} under `SSL_VERIFY_NONE` accepts
a MITM certificate. The emitter therefore keeps `SSL_VERIFY_PEER` and passes a
callback (`_mfb_tls_verify_cb`) that clears only 18/19/20 and returns 1, letting
verification continue into the name and date checks. Because the callback resets
the error to `X509_V_OK`, the post-handshake `SSL_get_verify_result == 0` check
is unchanged. **`openssl s_client` is not a valid probe for this** — it installs
its own callback returning 1, so it reports 62 where the emitted code reports 18.

**Schannel.** `SCH_CRED_MANUAL_CRED_VALIDATION` replaces
`SCH_CRED_AUTO_CRED_VALIDATION` (keeping `SCH_USE_STRONG_CRYPTO`) so
`InitializeSecurityContext` stops refusing outright, and
`CERT_CHAIN_POLICY_PARA::dwFlags` gains **only**
`CERT_CHAIN_POLICY_ALLOW_UNKNOWN_CA_FLAG`. Never the sibling
`..._IGNORE_INVALID_NAME`/`..._INVALID_DATE`, never clear `pwszServerName`, and
`dwError == 0` is still required.

**Network.framework.** A `sec_protocol_options_set_verify_block` block that
re-runs the *whole* SSL policy with the peer's own root as the anchor
(`SecTrustCopyCertificateChain`'s last entry →
`SecTrustSetAnchorCertificates` + `SecTrustSetAnchorCertificatesOnly(true)`),
under `SecPolicyCreateSSL(true, name)`. Never `complete(true)` unconditionally,
and never
`sec_protocol_options_set_peer_authentication_required(false)`.

**macOS is stricter than the other two, deliberately.** Apple enforces a
certificate *shape* policy: a TLS server certificate needs an `serverAuth`
extended key usage and a validity window under ~398 days, or
`SecTrustEvaluateWithError` refuses it as "not standards compliant" regardless of
anchors — the keychain exemption does not apply to a programmatic anchor and
there is no opt-out. A 10-year self-signed certificate therefore works on Linux
and Windows and fails on macOS *even with the flag*. Generate test and example
certificates with `-days 397 -addext extendedKeyUsage=serverAuth`.

## There is no `tls::wrap`, and the reason is macOS-specific

Upgrading an established `tcp::Socket` in place needs to adopt its fd. On macOS nothing supported can: Network.framework fixes TLS in `nw_parameters` at creation and cannot graft it onto a live connection; `nw_connection_create_with_connected_socket` is exported but declared in no SDK header and fails `ENETDOWN` for every parameter shape; Secure Transport can adopt an fd but is deprecated and rejects `kTLSProtocol13` (`errSSLIllegalParam`), capping at TLS 1.2. The system LibreSSL (`/usr/lib/libssl.48.dylib`) *can* do it at TLS 1.3 — measured — but ships no headers and the unversioned path deliberately aborts, so it is unsupported. Shipping `wrap` on Linux and Windows alone would let a program compile for five targets and fail at runtime on one, so the member exists nowhere (plan-110-D §C9). Do not reintroduce it on two platforms.

### What that costs downstream: no in-band TLS upgrade, ever

The absence is not only an API gap — it decides which network protocols can be an MFB
package at all. A protocol is implementable in pure MFBASIC only if TLS is established at
**connect time**, before any protocol bytes flow (`tls::connect` / `tls::accept`). A
protocol that negotiates encryption **in band**, on an already-open plaintext socket,
needs the wrap that cannot exist:

| Pure-MFB viable (TLS at connect) | Binding-package only (in-band upgrade) |
| --- | --- |
| HTTPS, `wss://` | PostgreSQL — SSLRequest, then handshake on the same socket |
| Redis (TLS-on-connect), MongoDB | MySQL — TLS after the initial handshake packet |
| gRPC | SMTP / IMAP / POP3 — `STARTTLS` |
| | FTPS (`AUTH TLS`), LDAP StartTLS |

Establish which side a protocol falls on before scoping a package for it. The failure mode
is not a compile error: the driver works, and simply cannot encrypt on macOS.

Note that a WebSocket package is on the *left*, despite "upgrade" appearing in RFC 6455 —
`wss://` completes the TLS handshake first and then speaks HTTP over it. An HTTP
`101 Switching Protocols` upgrade and a TLS upgrade are unrelated operations.

## Repository client transport security is per-URL, not per-hop

In `repository/src/client.rs`, `ensure_transport_security(repo_url)` validates ONLY the initial URL (https, or http-loopback for local dev). It is called at each network entry point (`get_json`/`post_json`/`fetch_blob`/…) but does NOT follow redirects, so a 302 target is never re-checked by it.

The one shared `reqwest::blocking::Client` is built once in `http_client()` (a `OnceLock`) — that is the ONLY place `connect_timeout` AND the redirect policy can be set. Per-hop transport enforcement therefore lives in `redirect_policy()` / `ensure_redirect_target()`, not in `ensure_transport_security`. The redirect guard is https-only and blocks private/loopback/link-local/CGNAT/unspecified IP literals (incl. IPv4-mapped IPv6) — otherwise a hostile registry 302 drives blind SSRF (169.254.169.254, 127.0.0.1, RFC-1918) or an https→http downgrade leak. Blob bytes stay SHA-256 checked and control-plane bodies signature-checked regardless; the redirect guard only closes the transport-level leak.

**The redirect guard is ORIGIN-BLIND, deliberately — so a credential-bearing request must not use it at all (bug-490).** `ensure_redirect_target` vets a hop's scheme and IP-LITERAL class, and its own doc says a hostname resolving to an internal address is out of scope. So `https://attacker.example/` — or `https://localhost:1/` — passes, and it has to, because a presigned blob URL is exactly that shape.

That is safe for a blob GET (bytes are content-address-verified afterwards) and unsafe for the control plane, because **the registry credential is a BODY field (`sessionToken`), not a header**. reqwest's cross-host stripping (`remove_sensitive_headers`: AUTHORIZATION, COOKIE, …) covers headers only, and a 307/308 preserves method and body. So a control-plane call answered with `307 Location: https://attacker.example/x` re-posts the session token — and for `/publish` the whole base64 `.mfp`, for `/machines/link` the sealed ident keypair.

**The invariant: a credential-bearing request never follows a redirect.** There are now TWO clients, both `OnceLock`s (a per-call `Client` would spawn a tokio runtime per request, which is why `http_client` is shared in the first place):

| client | policy | used by |
| --- | --- | --- |
| `http_client()` | `redirect_policy()` — vetted hops | `fetch_blob`, `blob_exists`, `get_json` |
| `no_redirect_client()` | `Policy::none()` | `post_json`, `put_blob` |

No control-plane route is documented to redirect, so refusing is free. `get_json` stays on the vetted client because no GET path carries a credential — the paths interpolate owner names and package idents only; check that before adding a route with a token in the URL.

Takeaway: if you add a new registry route or loosen networking, remember the initial-URL check and the redirect check are SEPARATE — enforce both, and pick the client by whether the request carries a credential. `reqwest::redirect::Policy::custom` closures track depth via `attempt.previous().len()` and reject a hop with `attempt.error(String)`.

## The `http` server parses strictly, the client leniently — and rejects early with a lingering close

`http` has TWO header parsers on purpose (bug-506/507). `__http_requestHeaderMap`
+ `__http_requestFraming` are the SERVER side: they FAIL on every
request-smuggling primitive (second `Content-Length`, `Content-Length` with
`Transfer-Encoding`, whitespace before the colon, obs-fold, a non-final or
doubled `chunked`, a non-digit `Content-Length`) and cap the field count and line
length (`ErrMessageTooLarge` → 431). `__http_headerMapFromHead` +
`__http_framingLength` + `__http_frameComplete` are the CLIENT side and stay
lenient (last-wins, substring `chunked`): the client always reads to EOF with
`Connection: close`, so a sloppy upstream cannot desync it, and tightening it
would fail `http::read` against real servers. Do not "unify" them.

The server read loop (`__http_readRequestNet`/`Tls`) is bounded three ways —
`tcp::setReadTimeout` per read, a `datetime::monotonicNanos` whole-request
deadline, and `__http_frameAdvance`'s caps — and REPORTS its outcome in a
`__http_ReadResult.status`; nothing in it raises. `handleRequest` still TRAPs
the whole read/parse/serialize, because before bug-507 one bad chunk-size line
raised straight out of the loop and exited the process (OS-51).

`__http_frameAdvance` is incremental: `scanFrom` resumes the `\r\n\r\n` search
three bytes back (a terminator can straddle two reads), and `cursor` resumes the
chunk walk at the last complete chunk boundary. The old predicate re-scanned
from offset 0 after every read (2 MiB → 0.7 s, 64 MiB ≈ 12 min).

An early rejection (408/413/431 before the client finished sending) must
`__http_linger*` before returning: closing with unread input queued makes the
kernel send RST, and a client that is still in `send` sees EPIPE/ECONNRESET
instead of the 4xx just written (measured: python `recv` raised
`ConnectionResetError` and never saw the 431). The drain is bounded (4 MiB, 500 ms
per read) and is NOT run after a complete request — waiting for the client's EOF
there would add latency to every exchange.

Response side: `__http_checkResponse` turns a handler response with a control
byte in its reason or a header name/value into a built-in 500 BEFORE
`__http_serializeHead`, whose own FAIL is only the fail-closed backstop. HTAB is
allowed in a value/reason (RFC 9110 field whitespace); everything else below
0x20, and DEL, is not.
