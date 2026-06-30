# read

Read available bytes from a connected TLS socket.

## Synopsis

```
tls::read(sock AS TlsSocket, maxBytes AS Integer) AS List OF Byte
```

## Package

`tls`

## Imports

```
IMPORT tls
```

## Description

`read` receives decrypted application data from a connected `TlsSocket` and
returns it as a `List OF Byte`. A single call performs one underlying TLS read:
it returns as soon as any plaintext is available rather than waiting to fill the
requested size, so the returned list is frequently shorter than `maxBytes`. The
socket must still be open. [[src/builtins/tls.rs:call_return_type_name]]

The call blocks until at least one byte of application data has been decrypted,
the peer closes its side of the TLS session, or the underlying read fails.
`maxBytes` bounds the size of a single read and the size of the returned list; it
does not request that exactly that many bytes be read. On success the returned
list always holds at least one byte.

Unlike a plain stream read that signals end of stream with a zero-length result,
`read` raises an error when the peer has closed the connection: there is no
empty-list sentinel. To consume a whole response, call `read` in a loop,
appending each result, and stop when an `ErrConnectionClosed` error is raised.
Use `tls::readText` when the peer sends UTF-8 text and a `String` is more
convenient than raw bytes.

The bytes are read into a freshly allocated `maxBytes` buffer and copied into a
new `List OF Byte`; the function has no other side effects and does not close the
socket. TLS is implemented on Linux by driving the system OpenSSL library
(`libssl.so.3`, falling back to `libssl.so.1.1`); the macOS backend drives
Network.framework through a synchronous bridge. If the TLS layer cannot be
initialized — neither OpenSSL library can be loaded, or a required symbol is
missing — `read` raises `ErrTlsFailed`. [[src/builtins/tls.rs:TLS_SOCKET_TYPE]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `TlsSocket` | A connected TLS socket to receive from, as returned by `tls::connect`. It must still be open; reading from a closed socket is an error (see Errors). |
| `maxBytes` | `Integer` | The maximum number of bytes to read in this call. Must be positive. It caps the length of the returned list but does not guarantee that many bytes are returned. |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The decrypted bytes received in this read, in the order they arrived, with length between `1` and `maxBytes` inclusive. End of stream is not reported as an empty list; it is reported as an `ErrConnectionClosed` error. [[src/builtins/tls.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `maxBytes` is not positive. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070004` | `ErrConnectionClosed` | The peer has closed the TLS session (an end-of-stream read), as reported by a zero-length read. [[src/target/shared/code/error_constants.rs:ERR_CONNECTION_CLOSED_CODE]] |
| `77070008` | `ErrTlsFailed` | The underlying TLS read fails, or the system OpenSSL library or a required symbol could not be loaded. [[src/target/shared/code/error_constants.rs:ERR_TLS_FAILED_CODE]] |
| `77010001` | `ErrOutOfMemory` | The `maxBytes` read buffer or the returned `List OF Byte` could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Read up to 4096 bytes from a connected TLS socket:

```
IMPORT tls

RES conn = tls::connect("example.com", 443)
tls::writeText(conn, "GET / HTTP/1.0" + Chr(13) + Chr(10) + Chr(13) + Chr(10))
LET chunk = tls::read(conn, 4096)
' conn is closed by lexical drop when this scope ends
```

Drain a TLS connection until the peer closes it:

```
IMPORT tls

RES conn = tls::connect("example.com", 443)
tls::writeText(conn, "GET / HTTP/1.0" + Chr(13) + Chr(10) + Chr(13) + Chr(10))
MUT response = [] AS List OF Byte
DO
  MATCH tls::read(conn, 4096)
    CASE Ok(chunk) : response = append(response, chunk)
    CASE Err(err) : EXIT DO
  END MATCH
LOOP
' conn is closed by lexical drop when this scope ends
```

## See also

- `mfb man tls readText`
- `mfb man tls write`
- `mfb man tls writeText`
- `mfb man tls connect`
- `mfb man tls close`
- `mfb man net read`
