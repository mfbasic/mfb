# poll

Test whether a TLS socket has application data ready to read, or wait for the
first ready socket among many.

## Synopsis

```
tls::poll(sock AS tls::TlsSocket) AS Boolean
tls::poll(sock AS tls::TlsSocket, timeoutMs AS Integer) AS Boolean
tls::poll(socks AS List OF RES tls::TlsSocket) AS tls::TlsSocket
tls::poll(socks AS List OF RES tls::TlsSocket, timeoutMs AS Integer) AS tls::TlsSocket
```

## Package

`tls`

## Imports

```
IMPORT tls
```

`tls` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/tls/mod.rs:register]]

## Description

`tls::poll` reports whether a connected `TlsSocket` is readable — that is, whether
a following `tls::read` or `tls::readText` can proceed without blocking. It returns
`TRUE` when application bytes are available (or the connection has reached a
terminal readable state — peer close or error — where a read returns promptly), and
`FALSE` when nothing became readable before the deadline. The socket is borrowed and
inspected only; no application data is consumed, so a `TRUE` result leaves the bytes
in place for the next read. [[src/codegen/builtins/tls/native/mod.rs:lower_tls_poll_helper]]

**Readiness includes bytes buffered inside the TLS layer, not just raw transport
state.** A single TLS record can carry many application bytes: one decrypt drains a
record and buffers the remainder, so a `TlsSocket` may hold decrypted bytes ready to
read while the underlying transport is idle. `tls::poll` accounts for this — it is
`TRUE` whenever the next read would return bytes, whether they are already buffered
or still on the wire. A plain fd-level poll would wrongly report "not ready" in the
buffered case. [[src/codegen/builtins/tls/native/openssl.rs:lower_tls_poll_openssl]]

`timeoutMs` bounds the wait, in milliseconds, following the language timeout
convention (see `mfb spec language builtin-functions` → "Timeout convention"). When
it is **omitted, `poll` blocks** until the socket becomes readable and then returns
`TRUE` (omit = unbounded). `0` is a non-blocking check that returns immediately with
the socket's current readiness. A positive value waits up to that long. A negative
`timeoutMs` is rejected with `ErrInvalidArgument`; a value above 2147483647 is
clamped to that. [[src/codegen/builtins/tls/mod.rs:SENTINEL]]

Behaviour is identical, per the convention, across all three TLS backends: macOS
Network.framework, Linux/BSD OpenSSL, and Windows Schannel.
[[src/codegen/builtins/tls/native/schannel_read_close.rs:lower_tls_poll]]

Given a `List OF RES tls::TlsSocket`, `tls::poll` becomes a **readiness multiplex**: it
blocks until at least one socket in the list is readable, then returns the first
ready one (lowest list index). The returned `TlsSocket` is a **borrowed** pointer —
an alias of a list element — so the list retains ownership and closes every socket
exactly once on scope exit; you may read, `return`, or `thread::transfer` through the
returned handle, but you do not close it. An empty list is rejected with
`ErrInvalidArgument`; because the multiplex yields a resource with no not-ready
value, expiry raises `ErrTimeout` (a producing call). The elements must be marked
`RES` (`List OF RES tls::TlsSocket`); a bare `List OF TlsSocket` is a compile error. The
buffered-readiness rule above applies per socket, so a socket holding decrypted bytes
with an idle transport is correctly reported ready by the multiplex.
[[src/codegen/builtins/tls/native/mod.rs:lower_tls_poll_list_helper]]

`tls::poll` complements the blocking `tls::read`: `poll` asks whether a read would
block right now, letting a program interleave its own work with a cooperatively
driven read loop.

## Overloads

**`tls::poll(sock AS tls::TlsSocket) AS Boolean`**

Blocks until the socket becomes readable, then returns `TRUE` (omitted `timeoutMs`
= unbounded wait).

**`tls::poll(sock AS tls::TlsSocket, timeoutMs AS Integer) AS Boolean`**

Waits at most `timeoutMs` milliseconds for the socket to become readable; `0` is a
non-blocking check.

**`tls::poll(socks AS List OF RES tls::TlsSocket) AS tls::TlsSocket`**

Blocks until one socket in `socks` is readable, then returns a borrowed pointer to
the first ready one. The list still owns and closes every socket.

**`tls::poll(socks AS List OF RES tls::TlsSocket, timeoutMs AS Integer) AS tls::TlsSocket`**

Waits at most `timeoutMs` milliseconds for one socket to become readable; `0` is a
single immediate scan. Expiry with none ready raises `ErrTimeout`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `TlsSocket` | An open TLS socket, as returned by `tls::connect` or `tls::accept`. It is borrowed and inspected for readiness only; no data is read and the handle is not consumed. [[src/codegen/registry/mod.rs:call_param_name_overloads]] |
| `socks` | `List OF RES tls::TlsSocket` | A non-empty list of open TLS sockets. Each is borrowed and inspected for readiness; the list keeps ownership. An empty list raises `ErrInvalidArgument`. [[src/codegen/registry/mod.rs:call_param_name_overloads]] |
| `timeoutMs` | `Integer` | Optional. Omit to block until a socket is readable; `0` is an immediate non-blocking check/scan; a positive value waits up to that many milliseconds, clamped to `2147483647`. Must not be negative. [[src/codegen/builtins/tls/mod.rs:SENTINEL]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | (scalar overload) `TRUE` when a following `tls::read`/`tls::readText` will not block — including buffered decrypted bytes with an idle transport, and terminal states where the read returns promptly. `FALSE` when nothing became readable before the deadline. [[src/codegen/builtins/tls/mod.rs:register]] |
| `TlsSocket` | (list overload) A **borrowed** pointer to the first ready socket (lowest list index). The list retains ownership and closes it; do not close the returned handle. [[src/codegen/builtins/tls/native/mod.rs:lower_tls_poll_list_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative, or (list overload) `socks` is empty. [[src/codegen/builtins/errorcode/mod.rs:ErrInvalidArgument]] |
| `77050008` | `ErrTimeout` | (list overload) the timeout expires with no socket ready. The scalar overload returns `FALSE` instead. [[src/codegen/builtins/errorcode/mod.rs:ErrTimeout]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed, or the underlying readiness check fails for a reason other than an interruption. [[src/codegen/builtins/errorcode/mod.rs:ErrResourceClosed]] |

## Examples

Check whether encrypted data is waiting without blocking (pass `0` for the immediate
check — omitting the timeout would instead block until the socket is readable):

```
IMPORT tls
IMPORT io

FUNC main AS Integer
  RES sock = tls::connect("example.com", 443)
  tls::writeText(sock, "GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")
  IF tls::poll(sock, 0) THEN
    io::print(tls::readText(sock, 4096))
  END IF
  tls::close(sock)
  RETURN 0
END FUNC
```

Wait until the peer responds, then read (the omitted timeout blocks):

```
IMPORT tls
IMPORT io

FUNC main AS Integer
  RES sock = tls::connect("example.com", 443)
  tls::writeText(sock, "GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")
  IF tls::poll(sock) THEN
    io::print(tls::readText(sock, 4096))
  END IF
  tls::close(sock)
  RETURN 0
END FUNC
```

Wait for the first ready socket among several (the readiness multiplex). The
returned socket is borrowed — the list still owns and closes both:

```
IMPORT tls
IMPORT io
IMPORT collections

FUNC main AS Integer
  RES a = tls::connect("example.com", 443)
  RES b = tls::connect("example.com", 443)
  MUT socks AS List OF RES tls::TlsSocket = []
  socks = collections::append(socks, a)
  socks = collections::append(socks, b)
  tls::writeText(b, "GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")
  RES ready AS tls::TlsSocket = tls::poll(socks)
  io::print(tls::readText(ready, 64))
  RETURN 0
END FUNC
```

## See also

- `mfb man tls read`
- `mfb man tls readText`
- `mfb man tls connect`
- `mfb man tls accept`
- `mfb man tls close`
- `mfb man net poll` — the plaintext-socket readiness query
- `mfb spec language builtin-functions` — the timeout convention
