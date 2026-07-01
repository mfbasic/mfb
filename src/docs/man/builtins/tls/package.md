# tls

Outbound TLS client connections and encrypted application-data transfer

## Synopsis

```
IMPORT tls
RES conn = tls::connect("example.com", 443)
tls::writeText(conn, "GET / HTTP/1.0" + Chr(13) + Chr(10) + Chr(13) + Chr(10))
LET reply = tls::readText(conn, 4096)
tls::close(conn)
```

## Description

The `tls` package opens outbound TLS client connections and reads and writes
encrypted application data over them. `tls::connect` resolves a host, opens a TCP
stream, performs a TLS client handshake, and verifies the peer's certificate
before returning a connected socket; `tls::read` and `tls::readText` receive
decrypted data; `tls::write` and `tls::writeText` send data; and `tls::close`
tears a connection down. The package is a client only — there is no listener or
accept side; use `net` for plain unencrypted TCP and UDP. [[src/builtins/tls.rs:is_tls_call]]

The package defines one built-in type. `TlsSocket` is a connected TLS client
stream: an opaque, owned, non-copyable resource handle. It is closed
automatically by lexical drop when its binding leaves scope, so `tls::close` is
needed only to release a handle earlier; unlike `net::close`, `tls::close`
consumes the handle and treats an already-closed socket as success rather than an
error. `TlsSocket` handles cannot be stored as collection elements or carried in
records. [[src/builtins/tls.rs:TLS_SOCKET_TYPE]] [[src/builtins/tls.rs:consumes_argument]]

Hosts are UTF-8 `String` values naming either a textual IP address or a name
passed to the system host resolver, which connects to the first resolved IPv4
address. The handshake negotiates TLS 1.2 or later against the system trust store
and always verifies the certificate chain and the expected server name; by
default the name is the host, but a non-empty `serverName` both selects the name
to validate and is sent as the TLS Server Name Indication (SNI) extension, which
is needed when connecting to a literal IP or a virtual host. Ports and the
`maxBytes` read cap are `Integer` values, and `maxBytes` must be positive. The
`timeoutMs` argument to `tls::connect` is `Integer` milliseconds but is advisory
on the current backend and does not yet bound the attempt. [[src/builtins/tls.rs:default_argument_padding]]

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
| `77010001` | `ErrOutOfMemory` | raised by `connect`, `read`, and `readText` when an internal allocation fails, such as a NUL-terminated host copy, a read or handshake buffer, the `TlsSocket` handle, or the `List OF Byte` or `String` holding a result [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77020004` | `ErrEncoding` | raised by `readText` when the received bytes are not valid UTF-8 [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77030004` | `ErrResourceClosed` | raised by `read`, `readText`, `write`, and `writeText` when the `TlsSocket` has already been closed [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77050002` | `ErrInvalidArgument` | raised by `read` and `readText` when `maxBytes` is not positive [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77070002` | `ErrAddressNotFound` | raised by `connect` when `host` cannot be resolved, including when it is malformed or has no address record [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_NOT_FOUND_CODE]] |
| `77070003` | `ErrNetworkFailed` | raised by `connect` when the socket cannot be created or the TCP connection cannot be established before the handshake begins, such as a refused or unreachable peer [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77070004` | `ErrConnectionClosed` | raised by `read` and `readText` when the peer has closed the TLS session (an end-of-stream read) [[src/target/shared/code/error_constants.rs:ERR_CONNECTION_CLOSED_CODE]] |
| `77070008` | `ErrTlsFailed` | raised by any function when the TLS layer cannot be initialized (the system OpenSSL library or a required symbol could not be loaded); by `connect` when the handshake fails (chain validation, server name mismatch, protocol negotiation, or a reset during the handshake); by `read` and `readText` when the underlying TLS read fails; by `write` and `writeText` when the underlying TLS write fails for any reason including a closed or reset peer; and by `close` when the session cannot be torn down [[src/target/shared/code/error_constants.rs:ERR_TLS_FAILED_CODE]] |
