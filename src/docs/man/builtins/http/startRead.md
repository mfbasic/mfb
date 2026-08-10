# startRead

Begin a non-blocking HTTP/1.1 GET-style exchange and return a drivable stream.

## Synopsis

```
http::startRead(url AS net::Url) AS RES http::Stream STATE PendingState
http::startRead(url AS net::Url, headers AS Map OF String TO String) AS RES http::Stream STATE PendingState
http::startRead(url AS net::Url, headers AS Map OF String TO String, method AS String) AS RES http::Stream STATE PendingState
```

## Package

`http`

## Imports

```
IMPORT net
IMPORT http
```

`IMPORT net` is required because the first argument is a `net::Url` (build one with
`net::toUrl`). The `Stream` union, its `PendingState`, and `Response` are provided
by `http`.

## Description

`startRead` opens a connection, writes a body-less request, and returns
immediately with a bound `http::Stream` — a resource union over the plaintext
(`net::Socket`) and TLS (`net::TlsSocket`) transports — carrying a fresh
`PendingState`. It does **not** wait for the reply. The caller then drives the
exchange without blocking its thread: test `http::ready`, call `http::pump` to
read whatever bytes are available, repeat until `http::done`, and parse with
`http::finish`. [[src/builtins/http_package.mfb:__http_startExchange]]

The transport is chosen from `url.scheme` exactly as `http::read` does: `https`
connects over the `tls` package (default port 443), anything else over plaintext
`net` (default port 80). The request is built by the same machinery as the
blocking client — `Connection: close` is always sent, `method` (default `GET`) is
uppercased, and the same control-byte rejection applies to every header name,
value, and the URL-derived request target and `Host`. The whole request is
written before `startRead` returns; `state.sentAll` is `TRUE`.
[[src/builtins/http_package.mfb:__http_buildRequest]] [[src/builtins/http_package.mfb:__http_normalizeMethod]]

The returned handle is a `RES http::Stream STATE PendingState`: an owned resource
whose STATE accumulates the response across pumps. It stays bound and open — the
socket is closed exactly once when the handle leaves scope — so a program reads
`state` through the handle while driving it. `http::read`/`http::write` are thin
blocking wrappers over this same core. [[src/builtins/http.rs:HTTP]]

`startRead` applies the 30-second connect deadline; the per-read deadline is a
matter for the drive loop (`http::pump` never blocks; the blocking wrappers'
internal readiness wait bounds a stalled peer). [[src/builtins/http_package.mfb:__HTTP_CONNECT_TIMEOUT_MS]]

## Overloads

**`http::startRead(url AS net::Url) AS RES http::Stream STATE PendingState`**

Starts a `GET` with no caller headers. [[src/builtins/http.rs:HTTP]]

**`http::startRead(url AS net::Url, headers AS Map OF String TO String) AS RES http::Stream STATE PendingState`**

Starts a `GET` with the supplied headers.

**`http::startRead(url AS net::Url, headers AS Map OF String TO String, method AS String) AS RES http::Stream STATE PendingState`**

Starts `method` (uppercased) with the supplied headers. The shorter overloads
default `headers` to an empty map and `method` to `GET`.
[[src/builtins/http.rs:default_argument_padding]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `url` | `net::Url` | The target URL. `url.scheme` selects transport (`https` → TLS on default port 443, otherwise plaintext on default port 80); `url.host`, `url.port`, `url.path`, and `url.query` form the connection and request target. |
| `headers` | `Map OF String TO String` | Optional request headers. Names matching `Host`/`User-Agent`/`Accept` override the defaults case-insensitively; others are appended. No name or value may contain a control byte. Defaults to an empty map. |
| `method` | `String` | Optional request method; uppercased before sending. Must be non-empty and contain no space. Defaults to `GET`. |

## Return value

| Type | Description |
| --- | --- |
| `RES http::Stream STATE PendingState` | The bound, open stream carrying a default `PendingState` (`sentAll = TRUE`, `closed = FALSE`, `raw = []`, `err = 0`). Drive it with `http::ready`/`http::pump`/`http::done` and parse with `http::finish`; it is closed once by its scope drop. [[src/builtins/http_package.mfb:PendingState]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `method` is empty or contains a space, or a caller header name/value or the URL-derived request target/`Host` contains a control byte (below `0x20`). [[src/builtins/errorcode.rs:ErrInvalidArgument]] |

Connect, DNS, write, and TLS failures propagate unchanged from the underlying
`net`/`tls` calls (for example `ErrAddressNotFound`, `ErrNetworkFailed`, or
`ErrTlsFailed`); read/framing/overflow failures surface later, from `http::pump`
and `http::finish`. [[src/builtins/http_package.mfb:__http_startExchange]]

## Examples

Drive a GET cooperatively, interleaving other work:

```
IMPORT net
IMPORT http
IMPORT io

SUB main()
  RES s AS http::Stream STATE PendingState = http::startRead(net::toUrl("http://example.com/"))
  WHILE http::done(s) = FALSE
    IF http::ready(s) THEN
      http::pump(s)
    END IF
    ' ... the caller's own work happens here, uninterrupted ...
  END WHILE
  LET r AS http::Response = http::finish(s)
  io::print(toString(r.status) & " " & r.reason)
END SUB
```

## See also

- `mfb man http ready`
- `mfb man http pump`
- `mfb man http done`
- `mfb man http finish`
- `mfb man http read`
- `mfb man net toUrl`
