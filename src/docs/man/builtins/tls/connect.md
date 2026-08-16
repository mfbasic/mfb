# connect

Open a TLS connection to a host and verify its certificate.

## Synopsis

```
tls::connect(host AS String, port AS Integer) AS tls::TlsSocket
tls::connect(host AS String, port AS Integer, timeoutMs AS Integer) AS tls::TlsSocket
tls::connect(host AS String, port AS Integer, timeoutMs AS Integer, serverName AS String) AS tls::TlsSocket
```

## Package

`tls`

## Imports

```
IMPORT tls
```

## Description

`connect` establishes an outbound TCP connection to `host` on `port`, performs a
TLS client handshake over it, and returns a connected `TlsSocket` resource. The
host is resolved with the system host resolver before connecting; the first
resolved IPv4 address is used. Once the socket is connected the handshake
negotiates TLS 1.2 or later — older protocol versions are refused — against the
system trust store loaded from the default certificate verification paths.

The peer's certificate is always verified: the certificate chain must validate
against the system trust store and the certificate must match the expected
server name. By default the expected name is `host`; supply a non-empty
`serverName` to validate against a different name and to send it as the TLS
Server Name Indication (SNI) extension, which is useful when connecting to a
literal IP address or to a virtual host whose certificate name differs from the
`host` argument. A handshake that fails for any reason — chain validation, name
mismatch, protocol negotiation, or a refused or reset connection during the
handshake — raises `ErrTlsFailed`, and the underlying socket is closed before
the error is returned.

`timeoutMs` follows the language timeout convention (see
`mfb spec language builtin-functions` → "Timeout convention"). When it is
**omitted the connect blocks** until it completes or the OS/TLS layer fails. `0`
is one immediate attempt: it succeeds if the connection and handshake complete at
once, otherwise it raises `ErrTimeout` without waiting. A positive value bounds
the attempt and raises `ErrTimeout` when it elapses. A negative `timeoutMs` raises
`ErrInvalidArgument`. All three backends implement it: the OpenSSL and Schannel
paths make the socket non-blocking and poll the connect to the deadline
(`poll`/`WSAPoll`), then bound the handshake with `SO_RCVTIMEO`/`SO_SNDTIMEO`; the
macOS path computes a `dispatch_time` deadline (`DISPATCH_TIME_NOW` for `0`,
`FOREVER` for the omitted form) and bounds the handshake the same way. **Host
resolution is not bounded** — the resolver call happens before the deadline
starts, so a slow DNS lookup can exceed `timeoutMs`.
[[src/codegen/builtins/tls/native/openssl.rs:connect_timeout]]

TLS is implemented on Linux by driving the system OpenSSL library (`libssl.so.3`,
falling back to `libssl.so.1.1`) so a single binary spans OpenSSL 1.1.1 and 3.x;
the macOS backend drives Network.framework through a synchronous bridge. If the
TLS layer cannot be initialized — neither OpenSSL library can be loaded, or a
required symbol is missing — `connect` raises `ErrTlsFailed`.

`TlsSocket` resources are closed by lexical drop at scope exit or explicitly with
`tls::close`. Read and write data with `tls::read`, `tls::readText`,
`tls::write`, and `tls::writeText`. [[src/codegen/builtins/tls/mod.rs:TLS_SOCKET_TYPE]]

## Overloads

**`tls::connect(host AS String, port AS Integer) AS tls::TlsSocket`**

Blocks until connected + handshaken (omitted `timeoutMs` = unbounded), validating
the certificate against `host`.

**`tls::connect(host AS String, port AS Integer, timeoutMs AS Integer) AS tls::TlsSocket`**

As above, bounded by a timeout in milliseconds (see Description).

**`tls::connect(host AS String, port AS Integer, timeoutMs AS Integer, serverName AS String) AS tls::TlsSocket`**

As above, but validates the certificate against `serverName` and sends it as the
SNI host name when `serverName` is non-empty. [[src/codegen/builtins/tls/mod.rs:register]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `host` | `String` | The host name or textual IP address of the peer. Resolved with the host resolver; a name that cannot be resolved raises an error (see Errors). Also used as the certificate validation and SNI name when `serverName` is omitted or empty. |
| `port` | `Integer` | The TCP port to connect to on the peer. |
| `timeoutMs` | `Integer` | Optional. The maximum time the connection and handshake may take, in milliseconds. Omit to block until it completes; `0` is one immediate attempt (`ErrTimeout` unless it completes at once); a positive value bounds it (`ErrTimeout` on elapse); a negative value raises `ErrInvalidArgument`. Host resolution happens first and is not counted against it. |
| `serverName` | `String` | Optional. When non-empty, the name the peer certificate must match and the host name sent in the TLS SNI extension, replacing `host` for validation. Defaults to the empty string, in which case `host` is used. |

## Return value

| Type | Description |
| --- | --- |
| `TlsSocket` | A connected `TlsSocket` resource whose certificate has been verified, ready for reading and writing. The `TlsSocket` is closed by lexical drop at scope exit unless closed earlier with `tls::close`. [[src/codegen/builtins/tls/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | Memory for a host C string, the read/handshake buffers, or the `TlsSocket` handle could not be allocated. |
| `77050008` | `ErrTimeout` | The connection or handshake did not complete before the deadline: immediately when `timeoutMs` is `0`, or after a positive `timeoutMs` elapsed. The omitted (unbounded) form never raises this. Host resolution is not counted against the deadline. |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative. |
| `77070002` | `ErrAddressNotFound` | `host` could not be resolved, including when it is malformed or has no address record. **Linux only:** the macOS backend reports every connection-establishment failure, an unresolvable host included, as `ErrTlsFailed`. |
| `77070003` | `ErrNetworkFailed` | The socket could not be created or the TCP connection could not be established before the TLS handshake begins (for example the peer refused the connection or is unreachable). |
| `77070008` | `ErrTlsFailed` | The TLS layer could not be initialized (the system OpenSSL library or a required symbol could not be loaded), or the handshake failed — certificate chain validation failure, server name mismatch, protocol negotiation failure, or a connection reset during the handshake. |

## Examples

Connect to an HTTPS server and validate its certificate:

```
IMPORT tls

SUB main()
  RES conn = tls::connect("example.com", 443)
  tls::writeText(conn, "GET / HTTP/1.0\r\n\r\n")
  LET response = tls::readText(conn, 4096)
  ' conn is closed by lexical drop when this scope ends
END SUB
```

Connect to a literal IP but validate against a named certificate via SNI:

```
IMPORT tls

SUB main()
  RES conn = tls::connect("93.184.216.34", 443, timeoutMs := 5000, serverName := "example.com")
  ' conn is closed by lexical drop when this scope ends
END SUB
```

## See also

- `mfb man tls read`
- `mfb man tls readText`
- `mfb man tls write`
- `mfb man tls writeText`
- `mfb man tls close`
- `mfb man net connectTcp`
- `mfb spec language builtin-functions` — the timeout convention
