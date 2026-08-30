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
Imports are not transitive and a package cannot re-export another's types, so
`IMPORT tcp` alone does not put `Address` in scope. The symptom is not obvious:
a binding annotated with an unimported record type fails later, in lowering, with
a message about an unknown storage class rather than about the import.

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
* **An already-closed handle is an error, not a no-op**, for `tcp` and `udp`.
  `tls::close` differs deliberately: it treats an already-closed handle as
  success, so closing and then letting the binding drop is safe on either.
* **The list form of `poll` returns a BORROWED element.** The list remains the
  owner and closes it once; the binding that receives it registers no close
  obligation. Classifying that bind as an owner double-closes the element.

`tcp::Socket` and `udp::Socket` are thread-sendable; the listeners are not (they
accept on their owning thread), and neither is a `tls::Socket` (a TLS session is
driven from the thread that owns it). No transport handle may be stored in a
record field; a collection element is allowed, and is what the list `poll` form
exists for.

## Streams versus datagrams

`tcp` and `tls` carry a byte stream. A read returns *whatever has arrived*, up to
`maxBytes` — never a promise of a whole message. A stream read stops wherever the
network divided the data, and that boundary need not be a character boundary,
which is why `read` is bytes-only in every package: decoding a partial read would
either guess or raise on valid traffic. `encoding::utf8Decode` does the decoding
once the caller has assembled a whole message. `write` is not symmetric — the
sender always knows what it has — so it takes a `String` as a second overload.

An empty read (`[]`) marks end of stream: the peer closed. It is not an error and
not a timeout.

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
* **The server presents, and does not request.** `tls::listen` loads a
  certificate chain and private key into a server context that every accepted
  connection borrows; it does not request or verify a client certificate.
* **The key may be PKCS#8 or PKCS#1**, on every platform, unencrypted. One PEM
  therefore serves every target.
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
