# write

Send raw bytes over a connected TLS socket.

## Synopsis

```
tls::write(sock AS TlsSocket, bytes AS List OF Byte) AS Nothing
```

## Package

`tls`

## Imports

```
IMPORT tls
```

## Description

`write` sends the contents of `bytes` as application data over a connected
`TlsSocket`, encrypting it through the negotiated TLS session. It writes the
whole list: the call loops over the underlying TLS write until every byte has
been accepted, so a successful return means all of `bytes` was handed to the TLS
layer, not merely the first chunk. The socket must still be open.
[[src/target/shared/code/tls/openssl.rs:lower_tls_write_helper]]

The bytes are taken from the list in order, starting at its first element. An
empty `bytes` list is a no-op: nothing is sent and the call succeeds without
touching the TLS layer. The function reads from the existing list buffer and
allocates nothing of its own; it has no side effects beyond the bytes it sends
and does not close the socket. [[src/target/shared/code/tls/macos.rs:lower_tls_write_macos]]

`write` returns `Nothing`; there is no short-write result to inspect, because a
partial write that cannot be completed is reported as an error rather than a
count. Use `tls::writeText` to send a `String` as UTF-8 without first converting
it to a `List OF Byte`, and `tls::read` or `tls::readText` to receive the peer's
reply.

TLS is implemented on Linux by driving the system OpenSSL library
(`libssl.so.3`, falling back to `libssl.so.1.1`) so a single binary spans
OpenSSL 1.1.1 and 3.x; the macOS backend drives Network.framework through a
synchronous bridge. If the TLS layer cannot be initialized — neither OpenSSL
library can be loaded, or a required symbol is missing — `write` raises
`ErrTlsFailed`. [[src/builtins/tls.rs:TLS_SOCKET_TYPE]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `TlsSocket` | A connected TLS socket to send on, as returned by `tls::connect`. It must still be open; writing to a closed socket is an error (see Errors). |
| `bytes` | `List OF Byte` | The application data to send, in order. The entire list is written before the call returns. An empty list sends nothing and succeeds. |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `write` returns no value. A successful return means every byte of `bytes` was accepted by the TLS layer. [[src/builtins/tls.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070008` | `ErrTlsFailed` | The underlying TLS write fails or cannot complete the whole payload, or the system OpenSSL library or a required symbol could not be loaded. [[src/target/shared/code/error_constants.rs:ERR_TLS_FAILED_CODE]] |

## Examples

Send a raw request over a connected TLS socket:

```
IMPORT tls
IMPORT strings

SUB main()
  RES conn = tls::connect("example.com", 443)
  LET request = strings::toBytes("GET / HTTP/1.0\r\n\r\n")
  tls::write(conn, request)
  LET reply = tls::readText(conn, 4096)
  ' conn is closed by lexical drop when this scope ends
END SUB
```

## See also

- `mfb man tls writeText`
- `mfb man tls read`
- `mfb man tls readText`
- `mfb man tls connect`
- `mfb man tls close`
- `mfb man net write`
