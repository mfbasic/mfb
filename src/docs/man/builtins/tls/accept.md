# accept

Accept one inbound connection and complete the server-side TLS handshake.

## Synopsis

```
tls::accept(listener AS TlsListener) AS TlsSocket
tls::accept(listener AS TlsListener, timeoutMs AS Integer) AS TlsSocket
```

## Package

`tls`

## Imports

```
IMPORT tls
```

## Description

`accept` takes the next inbound TCP connection on a `TlsListener`, runs the
**server side** of the TLS handshake using the listener's loaded certificate and
key, and returns a connected `TlsSocket`. The returned socket is
indistinguishable from a client `TlsSocket`: read and write it with `tls::read`,
`tls::readText`, `tls::write`, and `tls::writeText`, and close it with
`tls::close` or by lexical drop.

The `listener` is **borrowed**, not consumed: it stays open for the next
`accept`, so a server loops on one listener to serve many connections. The
accepted socket borrows the listener's shared server TLS context; closing the
socket never frees that context (the listener owns it and frees it once, when the
listener closes), so accepted sockets may be closed in any order while the
listener and its siblings stay live.

The optional `timeoutMs` bounds how long `accept` waits for both an inbound
connection and the handshake to complete. A positive value fails with
`ErrTimeout` if no connection arrives, or the handshake does not finish, within
that many milliseconds. `0` (the default when omitted) blocks until a connection
is ready. A handshake that fails — a client that is not speaking TLS, an
incompatible protocol, or a connection reset mid-handshake — raises
`ErrTlsFailed`, and the accepted connection is closed before the error is
returned; the listener stays open, so the server can continue accepting.

This version presents the server certificate but does not request or verify a
client certificate (no mutual TLS). [[src/builtins/tls.rs:consumes_argument]]

## Overloads

**`tls::accept(listener AS TlsListener) AS TlsSocket`**

Blocks until a connection is ready and the handshake completes.

**`tls::accept(listener AS TlsListener, timeoutMs AS Integer) AS TlsSocket`**

As above, but fails with `ErrTimeout` if a connection and handshake do not
complete within `timeoutMs` milliseconds. [[src/builtins/tls.rs:resolve_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `listener` | `TlsListener` | A listening `TlsListener` from `tls::listen`. Borrowed, not consumed: it remains open for further `accept` calls. |
| `timeoutMs` | `Integer` | Optional. The maximum time to wait for a connection and its handshake, in milliseconds. Defaults to `0`, which blocks indefinitely. |

## Return value

| Type | Description |
| --- | --- |
| `TlsSocket` | A connected `TlsSocket` for the accepted client, with the server handshake complete, ready for reading and writing. Byte-for-byte interchangeable with a client `TlsSocket`. Closed by lexical drop at scope exit unless closed earlier with `tls::close`. [[src/builtins/tls.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The `TlsSocket` handle for the accepted connection could not be allocated. |
| `77030004` | `ErrResourceClosed` | The `listener` has already been closed. |
| `77050008` | `ErrTimeout` | `timeoutMs` is positive and no connection arrived, or the handshake did not complete, before the deadline. |
| `77070003` | `ErrNetworkFailed` | The underlying `accept` failed. |
| `77070008` | `ErrTlsFailed` | The TLS layer could not be initialized, or the server-side handshake failed (the peer is not speaking TLS, protocol negotiation failed, or the connection reset during the handshake). |

## Type checking

`listener` must be a `TlsListener`; the optional `timeoutMs` must be `Integer`.
Passing a `TlsSocket` where a `TlsListener` is required is rejected at compile
time.

## Examples

Serve connections in a loop, one request/response each:

```
IMPORT tls
IMPORT io

SUB main()
  RES server = tls::listen("", 8443, "cert.pem", "key.pem")
  WHILE TRUE
    RES client = tls::accept(server)
    LET request = tls::readText(client, 4096)
    tls::writeText(client, "HTTP/1.0 200 OK\r\n\r\nhi")
    tls::close(client)
  WEND
END SUB
```

## See also

- `mfb man tls listen`
- `mfb man tls connect`
- `mfb man tls readText`
- `mfb man tls writeText`
- `mfb man tls close`
- `mfb man net accept`
