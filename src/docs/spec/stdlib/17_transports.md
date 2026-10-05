# Transports (`tcp`, `udp`, `tls`)

The model behind the three transport packages: what a socket handle is and who
closes it, what a read or a write actually promises, how the two `poll` forms
differ, and where each package's behaviour is genuinely the same as the others'
versus only similarly named. The per-function signature and parameter reference is
`./mfb man tcp` (and `udp`, `tls`); this topic specifies the behaviour behind it.

plan-110 split what used to be one `net` transport surface across these three
packages. `net` kept only the things that are not a connection — name resolution,
ICMP echo (`./mfb spec stdlib icmp`), URL parsing (`./mfb spec stdlib url`) — and
the shared `Address` record they all speak.

## The shared endpoint: `net::Address`

Every transport names an endpoint with the same value record, `net::Address`
(`host` as a textual IP, `port` as an Integer). One record type is what lets an
address flow between them: a result from `net::lookup`, the sender of a received
datagram, and a socket's own `localAddress`/`remoteAddress` are all the same
thing, and any of them can be handed to any transport's connect, bind or send.

**A file that names an `Address` must `IMPORT net` as well as its transport.**
Imports do not transitively bind package namespaces, so `IMPORT tcp` alone does
not make `net::Address` available as an explicit type name. Type exposure through
signatures is distinct from importing the owner's namespace; see
`./mfb spec language modules-and-packages`.
[[src/resolver/resolution.rs:resolve_type]]

Addresses are IPv4. `net::lookup` returns only IPv4 results, the emitters use
`AF_INET`, and an IPv6 literal is rejected with `ErrAddressInvalid` rather than
silently mis-parsed. [[src/codegen/builtins/net/func_lookup.rs]]

## Handles, ownership, and close

Each package owns its own resource types, package-qualified and mutually
non-substitutable: `tcp::Socket`, `tcp::Listener`, `udp::Socket`, `tls::Socket`,
`tls::Listener`. They share bare names on purpose — a "Socket" is a socket — so
every spelling that names one, including a `UNION` variant and a `MATCH CASE`
pattern, is package-qualified. A bare `Socket` names nothing.

The identities are distinct *types*, not labels: binding a `tcp::Socket` to a
`udp::Socket` is a type error, not a widening. That matters more than it sounds,
because the two records have different layouts and the confusion was silent
before plan-110-E closed it.

Ownership follows the language's general resource rules (`./mfb spec language
resource-management`): a handle is closed exactly once by its owning scope's
lexical drop, and the package's `close` exists only to release earlier.

Three specifics are transport-shaped:

* **`close` consumes.** It is the one call in each package that moves its
  argument. Using the handle afterwards is a compile error
  (`TYPE_USE_AFTER_MOVE`), not a runtime raise.
* **An already-closed handle is an error, not a no-op**, in `tcp`, `udp` and
  `tls` alike — §15's rule, not a per-transport choice. `tls::close` reported
  success instead until bug-525; the divergence was documented on both pages
  rather than reconciled. Closing and then letting the binding drop is still
  safe on every transport: a drop-close discards its failure (§15).
* **`listen`'s `backlog` defaults to `128` on both transports.** `tls::listen`
  defaulted it to `0` ("host default") until bug-525, so the two mirrors did not
  queue the same depth unless the argument was given.
* **The list form of `poll` returns a BORROWED element.** The list remains the
  owner and closes it once; the binding that receives it registers no close
  obligation. Classifying that bind as an owner double-closes the element.

Every transport handle is thread-sendable — `tcp::Socket`, `udp::Socket`,
`tcp::Listener`, `tls::Socket` and `tls::Listener` — so a server may bind on one
thread and accept on another, and hand each accepted connection to a worker.
`thread::transfer` **moves** the handle (§16): the receiving thread carries the
close obligation and the sender's binding is consumed, so one handle is never
driven from two threads at once. A transport handle may be stored in a record
field and as a collection element; the list `poll` form exists for the latter.
(This paragraph said a record field was forbidden until bug-523. §15.4 permits
one — `TYPE Holder { handle AS RES fs::File STATE Cursor }` is the spec's own
example — and `tests/rt-behavior/resources/record-res-field-export-rt` carries
such a record across a package boundary.)

## Streams versus datagrams

`tcp` and `tls` carry a byte stream. A read returns *whatever has arrived*, up to
`maxBytes` — never a promise of a whole message. A stream read stops wherever the
network divided the data, and that boundary need not be a character boundary,
which is why `read` is bytes-only in every package: decoding a partial read would
either guess or raise on valid traffic. `encoding::utf8Decode` does the decoding
once the caller has assembled a whole message. `write` is not symmetric — the
sender always knows what it has — so it takes a `String` as a second overload.

End of stream is a raise, not a value: once the peer has closed, `read` raises
`ErrConnectionClosed`, and a `read` that returns always holds at least one byte
(`rt-behavior/tcp/tcp-read-eof-raises-rt`, `rt-behavior/tls/tls-read-eof-raises-rt`).

On `tls` three events are the peer's close, and every backend reports all three as
`ErrConnectionClosed`: a `close_notify` (whether or not the peer also closes its
socket), the transport's end of stream without one, and a reset. How each backend
recognises them:

| backend | `ErrConnectionClosed` when |
|---|---|
| OpenSSL | `SSL_read` returns 0; or fails with `SSL_ERROR_SYSCALL` (5), `SSL_ERROR_ZERO_RETURN` (6), or `SSL_ERROR_SSL` (1) whose `ERR_peek_last_error` reason is `SSL_R_UNEXPECTED_EOF_WHILE_READING` (294, OpenSSL 3.x). Any other `SSL_ERROR_SSL` stays `ErrTlsFailed`. |
| Schannel | `DecryptMessage` returns `SEC_I_CONTEXT_EXPIRED` (a success status with no plaintext); `recv` returns 0; or `recv` fails with `WSAECONNRESET`, `WSAECONNABORTED` or `WSAESHUTDOWN`. |
| Network.framework | the receive completes with no content, or the connection is already `failed`/`cancelled`. |

[[src/codegen/builtins/tls/gen_openssl.rs:lower_tls_read_openssl]]
[[src/codegen/builtins/tls/gen_schannel.rs:SEC_I_CONTEXT_EXPIRED]]
[[src/codegen/builtins/tls/gen_schannel_read_close.rs:lower_tls_read]]
[[src/codegen/builtins/tls/gen_macos/client.rs:lower_tls_read_macos]]

A `tls` read that has raised `ErrConnectionClosed` leaves the socket finished in
both directions. Every later `read` raises it again without touching the transport
(Schannel would otherwise `recv` on a socket a half-closed peer holds open), and
every later `write` raises it before creating, encrypting or sending anything,
empty payload included, even when the peer is still connected and reading. The
state is a sticky per-socket flag, set on the read's `ErrConnectionClosed` exit
and checked first in `write`:

| backend | flag | carried by `thread::transfer` as |
|---|---|---|
| OpenSSL | record word `TLS_OFFSET_PEER_CLOSED` (48), zeroed by `connect` and `accept` | a declared `Verbatim` live slot |
| Schannel | `st::PEER_CLOSED` in the state block, inside the header both constructors zero | part of the copied state block |
| Network.framework | `CTX_RXEOF` in the `calloc`'d connection ctx, set with `stlr`, loaded with `ldar` | the ctx pointer |

[[src/codegen/builtins/tls/gen_shared.rs:TLS_OFFSET_PEER_CLOSED]]
[[src/codegen/builtins/tls/gen_schannel.rs:PEER_CLOSED]]
[[src/codegen/builtins/tls/gen_macos/mod.rs:CTX_RXEOF]]
[[src/codegen/builtins/tls/gen_openssl.rs:lower_tls_write_openssl]]
[[src/codegen/builtins/tls/gen_schannel_io.rs:lower_tls_write]]
[[src/codegen/builtins/tls/gen_macos/client.rs:lower_tls_write_macos]]

Without the flag the three backends would all accept such a write: a half-closed
peer (TLS 1.3 permits one) is still reading, so the bytes are delivered. The rule
is a product decision that the error is the same on every target, pinned by
`rt-behavior/tls/tls-write-after-read-closed-raises-rt`. It is `tls` only: a
`tcp::write` after `tcp::read` has reported end of stream is handed to the
kernel as usual.

A write to a peer that has gone away raises `ErrConnectionClosed` — the same
code both `read` calls carry at end of stream, on `tcp` and on `tls` alike, and
on every target. It is not reported on the first such write: that one is accepted
by the local OS (or by the TLS layer), and only a later write in the same loop
raises, so a write is never a delivery receipt. `ErrTlsFailed` stays what it has
always meant on `tls` — a handshake, certificate or protocol failure — and a
write that reaches a `setWriteTimeout` deadline stays `ErrTimeout`. Each backend
had to be taught this separately: left alone, every one of them reports whatever
transport error it happens to produce (bug-467 for OpenSSL, bug-483 for
Network.framework and Schannel, which named the identical event `ErrTlsFailed`
and `ErrNetworkFailed`).

`udp` carries datagrams, and the difference is not a detail:

* Boundaries are preserved exactly. One `send` becomes one `receive`, never split
  and never merged with a neighbour.
* A zero-length datagram is ordinary and arrives as `bytes` of length 0. It is
  **not** end-of-stream — UDP has no such concept.
* There is no connection, ordering, retransmission or delivery confirmation. A
  lost datagram is simply never received, and a successful `send` means only that
  the local OS accepted it.
* `receive` reports the sender in `Datagram.from`, which is why `udp` has no
  `remoteAddress`: a connectionless socket has no single peer.

## `poll`: two overloads, two conventions

Every transport's `poll` is overloaded on its first argument, and the two forms
sit on opposite sides of the timeout convention (`./mfb spec language
builtin-functions`):

| Form | Returns | Convention | Deadline unmet |
|---|---|---|---|
| `poll(sock[, timeoutMs])` | `Boolean` | readiness query | `FALSE` |
| `poll(socks[, timeoutMs])` | the first ready socket, **borrowed** | producing call | `ErrTimeout` |

The list form takes a `List OF RES <pkg>::Socket` and answers with the
lowest-indexed ready element. An empty list raises `ErrInvalidArgument`: there is
nothing to wait on. A list of another package's sockets is a type error.

## Timeouts

All three packages obey the one language timeout convention rather than a
per-package variant: omitted blocks, `0` performs one immediate attempt, a
positive value bounds the wait, and a negative value raises
`ErrInvalidArgument`. Expiry raises exactly one error, `ErrTimeout`, for every
producing call.

`setReadTimeout`/`setWriteTimeout` bind a socket's *subsequent* operations rather
than waiting themselves. A fresh socket is unbounded and the setter can only
bound, so unbounded is not restorable through it.

## TLS specifics

`tls` is the encrypted stream: the same read/write/poll/close model as `tcp`,
plus a handshake and a peer identity.

* **The client verifies.** `tls::connect` validates the server's chain against
  the host trust store and checks the name; a chain it cannot verify raises
  rather than connecting. The optional `serverName` sets SNI, defaulting to the
  host.
* **One narrow opt-out, off by default.** The optional `Boolean`
  `allowSelfSigned` accepts a chain that fails *only* because its root is not in
  the host trust store — a self-signed certificate, or a private CA. It is not a
  "skip verification" switch: the server name, the certificate validity dates and
  the TLS 1.2 floor remain enforced on every backend, so a name mismatch or an
  expired certificate still raises. It defaults to `FALSE`, so no existing call
  changes behaviour; it cannot be set by environment variable or manifest key, so
  it is always visible at the call site; and `mfb audit` reports each use as
  `AUDIT-TLS-RELAXED-TRUST`. `http::` does not expose it — HTTPS through `http::`
  always verifies fully.
* **The server presents, and does not request.** `tls::listen` loads a
  certificate chain and private key into a server context that every accepted
  connection borrows; it does not request or verify a client certificate.
* **The key may be PKCS#8 or PKCS#1**, on every platform, unencrypted. One PEM
  therefore serves every target.
* **`tls::accept`'s `timeoutMs` bounds two steps, each by the full value.** The
  wait for an inbound connection is bounded by `timeoutMs`; once a connection
  is taken, its handshake is bounded by `timeoutMs` again, measured from that
  moment. Time the wait used is not taken from the handshake, so one call can
  take longer than `timeoutMs`. The timeout convention applies to each
  step: omitted blocks in both, `0` is one immediate attempt at each (a
  handshake essentially never completes at once, so `0` raises `ErrTimeout`
  even when a client is queued), and expiry of either step raises
  `ErrTimeout`. Each step is one absolute deadline, so a peer cannot extend
  the handshake by sending a little at a time, and one call takes at most
  about `2 × timeoutMs`. The mechanism differs by backend. On macOS the
  connection wait and the handshake wait are each a `dispatch_semaphore_wait`
  against an absolute deadline, and the handshake deadline is computed afresh
  when the connection is popped from the listener's ring.
  [[src/codegen/builtins/tls/gen_macos/server.rs:emit_accept_deadline]]
  [[src/codegen/builtins/tls/gen_macos/server.rs:lower_tls_accept_macos]]
  On Linux the connection wait is `poll` with `timeoutMs`; a bounded handshake
  then runs with the accepted fd non-blocking, calling `SSL_accept` and, on
  `WANT_READ`/`WANT_WRITE`, polling for the time left before a
  `CLOCK_MONOTONIC` deadline taken when the connection was accepted. The fd's
  blocking mode is restored afterwards. `0` makes one non-blocking
  `SSL_accept` attempt.
  [[src/codegen/builtins/tls/gen_openssl.rs:lower_tls_accept_openssl]]
  [[src/codegen/builtins/tls/gen_openssl.rs:emit_monotonic_ms]]
  On Windows the connection wait is `WSAPoll`; before every handshake `recv`
  and every `AcceptSecurityContext` pass that may send, `SO_RCVTIMEO`/`SO_SNDTIMEO`
  are re-armed to the time left before a `GetTickCount64` deadline (`0` is a
  1 ms deadline, since Winsock reads a zero timeout as infinite), and cleared
  after the handshake.
  [[src/codegen/builtins/tls/gen_schannel_server.rs:lower_tls_accept]]
  [[src/codegen/builtins/tls/gen_schannel_server.rs:emit_hs_refresh]]
* **A mismatched cert/key pair is not rejected at listen.** No backend verifies
  the pair while building the credential; the mismatch surfaces as `ErrTlsFailed`
  from the first `tls::accept`.
* **The server context is owned by the `Listener` and borrowed by each accepted
  `Socket`.** Closing an accepted socket never frees it; it is released exactly
  once, when the listener closes.

There is deliberately **no** `tls::wrap`: upgrading an established `tcp::Socket`
in place would require adopting its descriptor, and macOS exposes no supported
API that can. Shipping it on Linux and Windows alone would make a program compile
everywhere and fail at runtime on one target, so the member exists nowhere.

Each platform uses one TLS provider, not a mixture: Network.framework on macOS,
OpenSSL on Linux, Schannel on Windows.

## See Also

* ./mfb man tcp — the per-function API (and `udp`, `tls`)
* ./mfb spec language resource-management — the ownership rules these follow
* ./mfb spec language builtin-functions — the timeout convention
* ./mfb spec stdlib http — the client and server built on these transports
* ./mfb spec stdlib icmp — `net::ping`, the other half of what `net` kept
