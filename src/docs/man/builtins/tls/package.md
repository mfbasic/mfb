# tls

TLS client connections, TLS termination, and encrypted application-data transfer

## Synopsis

```
IMPORT tls
RES conn = tls::connect("example.com", 443)
tls::writeText(conn, "GET / HTTP/1.0" + Chr(13) + Chr(10) + Chr(13) + Chr(10))
LET reply = tls::readText(conn, 4096)
tls::close(conn)
```

```
IMPORT tls
RES server = tls::listen("", 8443, "cert.pem", "key.pem")
RES client = tls::accept(server)
LET request = tls::readText(client, 4096)
tls::writeText(client, "hi")
tls::close(client)
```

## Description

The `tls` package opens outbound TLS client connections, terminates inbound TLS
connections, and reads and writes encrypted application data over both.
`tls::connect` resolves a host, opens a TCP stream, performs a TLS client
handshake, and verifies the peer's certificate before returning a connected
socket. `tls::listen` binds a local port and loads a server certificate and key,
and `tls::accept` accepts one inbound connection and completes the server-side
handshake, returning a socket that is byte-for-byte interchangeable with a client
socket. `tls::read` and `tls::readText` receive decrypted data; `tls::write` and
`tls::writeText` send data; and `tls::close` tears down a socket or a listener.
For plain unencrypted TCP and UDP, use `net`. [[src/builtins/tls.rs:is_tls_call]]

The package defines two built-in types. `TlsSocket` is a connected TLS stream —
either an outbound client connection from `tls::connect` or an accepted server
connection from `tls::accept`. `TlsListener` is a bound, listening server
endpoint from `tls::listen` that owns the loaded server TLS context; `tls::accept`
draws connections from it. Both are opaque, owned, non-copyable resource handles.
Each is closed automatically by lexical drop when its binding leaves scope, so
`tls::close` is needed only to release a handle earlier; unlike `net::close`,
`tls::close` consumes the handle and treats an already-closed handle as success
rather than an error. Neither handle type is thread-sendable, and neither can be
stored as a collection element or carried in a record.
[[src/builtins/tls.rs:TLS_SOCKET_TYPE]] [[src/builtins/tls.rs:consumes_argument]]

The server's TLS context is owned by the `TlsListener` and borrowed by every
`TlsSocket` that `tls::accept` returns from it: closing an accepted socket never
frees the shared context, which is released exactly once when the listener
closes. Accepted sockets may therefore be closed in any order, and the listener
may be closed while accepted sockets are still live. The server presents its
certificate but does not request or verify a client certificate — there is no
mutual TLS, session resumption, ALPN, or SNI-based certificate selection in this
version. [[src/target/shared/code/tls/openssl.rs:lower_tls_close_helper]]

Hosts are UTF-8 `String` values naming either a textual IP address or a name
passed to the system host resolver, which connects to the first resolved IPv4
address. The handshake negotiates TLS 1.2 or later against the system trust store
and always verifies the certificate chain and the expected server name; by
default the name is the host, but a non-empty `serverName` both selects the name
to validate and is sent as the TLS Server Name Indication (SNI) extension, which
is needed when connecting to a literal IP or a virtual host. Ports and the
`maxBytes` read cap are `Integer` values, and `maxBytes` must be positive. The
`timeoutMs` argument to `tls::connect` is `Integer` milliseconds; a positive
value bounds the connection and handshake and raises `ErrTimeout` when it
elapses, while `0` means no bound. Host resolution runs before the deadline
starts and is not counted against it. [[src/target/shared/code/tls/openssl.rs:connect_timeout]]

The read and write functions come in paired byte/text forms: the byte form
transfers a `List OF Byte` verbatim, while the text form transfers a `String`'s
UTF-8 bytes directly and validates received bytes as UTF-8. Each read performs
one underlying TLS read and returns as soon as any plaintext is available, so a
result is frequently shorter than `maxBytes` and never empty on success; end of
stream is reported as an error rather than an empty result, so read in a loop
until the connection is closed. Each write transmits the entire buffer, looping
internally to resend any portion a single TLS write did not accept. TLS is
implemented on Linux by driving the system OpenSSL library (`libssl.so.3`,
falling back to `libssl.so.1.1`) so a single binary spans OpenSSL 1.1.1 and 3.x;
the macOS backend drives Network.framework through a synchronous bridge. If the
TLS layer cannot be initialized — neither library can be loaded, or a required
symbol is missing — the call fails. Unlike `net`, the `tls` functions map every
underlying read or write failure — a closed peer, a reset connection, or any
other SSL error during transfer — to a single TLS error rather than
distinguishing timeouts and closes. [[src/target/shared/code/tls/openssl.rs:lower_tls_read_helper]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | raised by `connect`, `listen`, `accept`, `read`, and `readText` when an internal allocation fails, such as a NUL-terminated host/path copy, a read or handshake buffer, a `TlsSocket`/`TlsListener` handle, or the `List OF Byte` or `String` holding a result [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77020004` | `ErrEncoding` | raised by `readText` when the received bytes are not valid UTF-8 [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77030004` | `ErrResourceClosed` | raised by `read`, `readText`, `write`, and `writeText` when the `TlsSocket` has already been closed, and by `accept` when the `TlsListener` has already been closed [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77050002` | `ErrInvalidArgument` | raised by `read` and `readText` when `maxBytes` is not positive [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050008` | `ErrTimeout` | raised by `connect` when a positive `timeoutMs` elapses before the connection and handshake complete (host resolution is not counted against it), and by `accept` when a positive `timeoutMs` elapses before a connection arrives and its handshake completes [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |
| `77070001` | `ErrAddressInvalid` | raised by `listen` when `host`/`port` cannot be resolved to a bindable local endpoint — **Linux only**; see the platform note below [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77070002` | `ErrAddressNotFound` | raised by `connect` when `host` cannot be resolved, including when it is malformed or has no address record — **Linux only**; see the platform note below [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_NOT_FOUND_CODE]] |
| `77070003` | `ErrNetworkFailed` | raised by `connect` when the socket cannot be created or the TCP connection cannot be established before the handshake begins; by `listen` when the listening socket cannot be created, bound, or set to listen; and by `accept` when the underlying accept fails [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77070004` | `ErrConnectionClosed` | raised by `read` and `readText` when the peer has closed the TLS session (an end-of-stream read) [[src/target/shared/code/error_constants.rs:ERR_CONNECTION_CLOSED_CODE]] |
| `77070008` | `ErrTlsFailed` | raised by any function when the TLS layer cannot be initialized (the system OpenSSL library or a required symbol could not be loaded); by `connect` when the client handshake fails; by `listen` when the server certificate or key cannot be loaded or the key does not match the certificate; by `accept` when the server handshake fails; by `read` and `readText` when the underlying TLS read fails; by `write` and `writeText` when the underlying TLS write fails; and by `close` when a session or listener cannot be torn down [[src/target/shared/code/error_constants.rs:ERR_TLS_FAILED_CODE]] |

Two of the codes above are raised only by the Linux (OpenSSL) backend. The macOS
(Network.framework) backend collapses every connection-establishment and bind
failure — an unresolvable host included — into `ErrTlsFailed`, so a program that
branches on `ErrAddressNotFound` or `ErrAddressInvalid` takes a different branch
there. Branch on `ErrTlsFailed` as well when the behavior must be identical on
both platforms. [[src/target/shared/code/tls/macos/client.rs:lower_tls_connect_macos]]
