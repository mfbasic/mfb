# close

Close a TLS connection and release its OS handle.

## Synopsis

```
tls::close(sock AS TlsSocket) AS Nothing
```

## Package

`tls`

## Imports

```
IMPORT tls
```

## Description

`close` shuts down a connected `TlsSocket` and releases the resources behind it.
On Linux it performs an orderly TLS shutdown and frees the OpenSSL objects
(`SSL_shutdown`, `SSL_free`, `SSL_CTX_free`) before closing the underlying socket
file descriptor; on macOS it cancels the Network.framework connection. After a
successful return the socket is marked closed and must not be used again — any
later `tls::` call that takes the same value raises an error rather than touching
a stale handle. [[src/target/shared/code/tls/openssl.rs:lower_tls_close_helper]] [[src/target/shared/code/tls/macos.rs:lower_tls_close_macos]]

`close` consumes the `TlsSocket` it is given: the value is moved into the call and
cannot be referenced afterward. [[src/builtins/tls.rs:consumes_argument]] The call is
idempotent with respect to a socket that is already closed — closing a socket
whose closed flag is already set does nothing and returns successfully — so
closing a socket and then letting it drop is safe, and a socket closed by an
earlier scope-drop reports success rather than an error. This differs from
`net::close`, which treats an already-closed resource as an error.
[[src/target/shared/code/tls/openssl.rs:lower_tls_close_helper]]

Closing is otherwise automatic. Every `TlsSocket` is closed by lexical drop when
the binding that holds it leaves scope. [[src/builtins/tls.rs:resource_close_function]]
Call `tls::close` only when the connection must be torn down earlier than that —
for example to let a peer observe the end of the stream promptly, or to bound the
number of connections a long-running program holds open at once.

TLS is implemented on Linux by driving the system OpenSSL library (`libssl.so.3`,
falling back to `libssl.so.1.1`) so a single binary spans OpenSSL 1.1.1 and 3.x;
the macOS backend drives Network.framework through a synchronous bridge. If
neither library can be loaded, or a required symbol is missing while tearing down
the session, `close` raises `ErrTlsFailed`; the underlying socket file descriptor
is still closed in that case. [[src/target/shared/code/tls/openssl.rs:lower_tls_close_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `TlsSocket` | The connected TLS socket to close, as returned by `tls::connect`. The value is consumed by the call and cannot be used afterward. Closing a socket that is already closed is harmless and returns successfully. |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `close` returns no value. After a successful return the TLS session has been shut down, the OS handle released, and the socket marked closed; it must not be used again. [[src/builtins/tls.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77070008` | `ErrTlsFailed` | The system OpenSSL library (or, on macOS, Network.framework) or a required symbol could not be loaded while tearing down the session. The underlying socket file descriptor is still closed in this case. [[src/target/shared/code/error_constants.rs:ERR_TLS_FAILED_CODE]] |

## Examples

Close a TLS connection explicitly once the exchange is complete:

```
IMPORT tls

RES conn = tls::connect("example.com", 443)
tls::writeText(conn, "GET / HTTP/1.0" + Chr(13) + Chr(10) + Chr(13) + Chr(10))
LET response = tls::readText(conn, 4096)
tls::close(conn)
```

Close each connection inside a loop so connections are not held open:

```
IMPORT tls

FOR EACH host IN hosts
  RES conn = tls::connect(host, 443)
  tls::writeText(conn, "PING")
  tls::close(conn)
NEXT
```

## See also

- `mfb man tls connect`
- `mfb man tls read`
- `mfb man tls readText`
- `mfb man tls write`
- `mfb man tls writeText`
- `mfb man net close`
