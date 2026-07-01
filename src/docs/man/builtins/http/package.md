# http

Blocking HTTP/1.1 client layered on the native `net` and `tls` packages

## Synopsis

```
IMPORT net
IMPORT http
LET r = http::read(net::toUrl("http://example.com/"))
LET p = http::write(net::toUrl("http://example.com/items"), body)
LET v = collections::getOr(r.headers, "content-type", "")
```

## Description

The `http` package is a blocking HTTP/1.1 client. It adds no platform code:
transport is provided by the native `net` and `tls` packages, so an `http://`
request goes over `net` and an `https://` request goes over `tls`, and the
package works on both Linux and macOS. A program writes `IMPORT http`; because
the public surface takes a `net::Url`, callers also `IMPORT net`. They never
need `IMPORT tls` — the TLS branch stays sealed inside `http`. [[src/builtins/http_package.mfb:__http_exchange]]

`http::read` performs a body-less request (GET by default; also HEAD, DELETE,
OPTIONS) and `http::write` performs a request that carries a body (POST by
default; also PUT, PATCH), sending the body as UTF-8 with a matching
`Content-Length`. Both accept an optional `Map OF String TO String` of extra
request headers and an optional trailing method name; the method is uppercased
before it is sent. Each call opens one connection, exchanges exactly one request
and response, and closes the connection before returning, so no socket handle
escapes. [[src/builtins/http_package.mfb:__http_read]]

Both functions return an `http::Response`, an ordinary copyable value record
with fields `status` (Integer), `reason` (String), `httpVersion` (String),
`headers` (Map OF String TO String), `body` (String), and `ok` (Boolean, TRUE
when `status` is in `200..299`). Because the record holds no resource handle, a
`Response` can be returned, copied, and sent across threads. [[src/builtins/http_package.mfb:Response]]

The `headers` field is a standard map whose field names are lowercased (HTTP
field names are case-insensitive), with duplicate fields collapsed last-wins.
Read it with the ordinary `collections` accessors — for example
`collections::getOr(resp.headers, "content-type", "")` or
`collections::hasKey(resp.headers, "location")`. There is no dedicated header
function. [[src/builtins/http_package.mfb:__http_parseResponse]]

The client always supplies `Host`, `User-Agent` (`mfb-http/1`), `Accept`
(`*/*`), and `Connection: close`, plus `Content-Length` for `http::write`. A
caller `headers` entry adds or overrides any request header case-insensitively,
except the reserved framing headers `Content-Length` and `Connection`, which are
controlled by the client to preserve framing. The response is read to end of
stream and parsed: the status line fills `status`/`reason`/`httpVersion`, header
lines fill the `headers` map, and the body is de-chunked when
`Transfer-Encoding` is chunked, otherwise taken as received (a 204 or 304
response carries no body). The body is decoded as UTF-8 text. Redirects are
returned as-is (`ok` is FALSE and the location is in `resp.headers`) rather than
followed. [[src/builtins/http_package.mfb:__http_buildRequest]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | raised by `read` and `write` when the method is empty or contains a space character [[src/builtins/http_package.mfb:__http_normalizeMethod]] |
| `77050003` | `ErrInvalidFormat` | raised by `read` and `write` when the response status line, header block, or chunked framing is malformed [[src/builtins/http_package.mfb:__http_parseStatusLine]] |
| `77050010` | `ErrOverflow` | raised by `read` and `write` when the accumulated response exceeds the internal 64 MiB size cap [[src/builtins/http_package.mfb:__http_exchangeTcp]] |

Transport failures from `net` and `tls` are propagated unchanged; a clean end of
stream terminates an EOF-framed body and is not an error.
