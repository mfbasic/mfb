# readText

Read available bytes from a connected TLS socket as UTF-8 text.

## Synopsis

```
tls::readText(sock AS TlsSocket, maxBytes AS Integer) AS String
```

## Package

`tls`

## Imports

```
IMPORT tls
```

## Description

`readText` receives decrypted application data from a connected `TlsSocket` and
returns it decoded as a UTF-8 `String`. A single call performs one underlying TLS
read: it returns as soon as any plaintext is available rather than waiting to
fill the requested size, so the returned `String` is frequently built from fewer
than `maxBytes` bytes. The socket must still be open.
[[src/builtins/tls.rs:call_return_type_name]]

The call blocks until at least one byte of application data has been decrypted,
the peer closes its side of the TLS session, or the underlying read fails.
`maxBytes` bounds the number of bytes read in this call and the number of bytes
decoded into the result; it does not request that exactly that many bytes be
read. On success the returned `String` is built from at least one byte.

Unlike a plain stream read that signals end of stream with a zero-length result,
`readText` raises an error when the peer has closed the connection: there is no
empty-`String` sentinel. To consume a whole response, call `readText` in a loop,
appending each result, and stop when an `ErrConnectionClosed` error is raised.
[[src/target/shared/code/tls/openssl.rs:lower_tls_read_helper]]

The decrypted bytes are validated as UTF-8 before being returned; invalid UTF-8
raises an `ErrEncoding` error. Because a single TLS read may split a multi-byte
UTF-8 sequence across calls, use `tls::read` instead when the peer sends raw
binary data, or when you need to reassemble bytes spanning multiple reads before
decoding. [[src/target/shared/code/tls/openssl.rs:emit_call_validate_utf8]]

The bytes are read into a freshly allocated `maxBytes` buffer and copied into a
new `String`; the function has no other side effects and does not close the
socket. TLS is implemented on Linux by driving the system OpenSSL library
(`libssl.so.3`, falling back to `libssl.so.1.1`); the macOS backend drives
Network.framework through a synchronous bridge. If the TLS layer cannot be
initialized — neither OpenSSL library can be loaded, or a required symbol is
missing — `readText` raises `ErrTlsFailed`. [[src/builtins/tls.rs:TLS_SOCKET_TYPE]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `TlsSocket` | A connected TLS socket to receive from, as returned by `tls::connect`. It must still be open; reading from a closed socket is an error (see Errors). |
| `maxBytes` | `Integer` | The maximum number of bytes to read in this call. Must be positive. It caps the number of bytes received before decoding but does not guarantee that many bytes are returned. |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decrypted bytes received in this read, decoded as UTF-8, in the order they arrived. The `String` is built from between `1` and `maxBytes` bytes inclusive. End of stream is not reported as an empty `String`; it is reported as an `ErrConnectionClosed` error. [[src/builtins/tls.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `maxBytes` is not positive. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070004` | `ErrConnectionClosed` | The peer has closed the TLS session (an end-of-stream read), as reported by a zero-length `SSL_read`. [[src/target/shared/code/error_constants.rs:ERR_CONNECTION_CLOSED_CODE]] |
| `77070008` | `ErrTlsFailed` | The underlying TLS read fails, or the system OpenSSL library or a required symbol could not be loaded. [[src/target/shared/code/error_constants.rs:ERR_TLS_FAILED_CODE]] |
| `77020004` | `ErrEncoding` | The received bytes are not valid UTF-8. [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77010001` | `ErrOutOfMemory` | The `maxBytes` read buffer or the returned `String` could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Read up to 4096 bytes of text from a connected TLS socket:

```
IMPORT tls

RES conn = tls::connect("example.com", 443)
tls::writeText(conn, "GET / HTTP/1.0" + Chr(13) + Chr(10) + Chr(13) + Chr(10))
LET reply = tls::readText(conn, 4096)
' conn is closed by lexical drop when this scope ends
```

Drain a TLS connection until the peer closes it:

```
IMPORT tls

RES conn = tls::connect("example.com", 443)
tls::writeText(conn, "GET / HTTP/1.0" + Chr(13) + Chr(10) + Chr(13) + Chr(10))
MUT response AS String = ""
DO
  MATCH tls::readText(conn, 4096)
    CASE Ok(chunk) : response = response & chunk
    CASE Err(err) : EXIT DO
  END MATCH
LOOP
' conn is closed by lexical drop when this scope ends
```

## See also

- `mfb man tls read`
- `mfb man tls write`
- `mfb man tls writeText`
- `mfb man tls connect`
- `mfb man tls close`
- `mfb man net readText`
