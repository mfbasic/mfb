# http

Blocking HTTP/1.1 client and single-threaded server layered on the native `net` and `tls` packages

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
`headers` (Map OF String TO String), `body` (List OF Byte), and `ok` (Boolean,
TRUE when `status` is in `200..299`). The `body` is raw bytes so binary payloads
survive intact; decode it to text with `toString(resp.body)`. Because the record
holds no resource handle, a `Response` can be returned, copied, and sent across
threads. The same `Response` record is used by the server (below). [[src/builtins/http_package.mfb:Response]]

The `headers` field is a standard map whose field names are lowercased (HTTP
field names are case-insensitive), with duplicate fields collapsed last-wins.
Read it with the ordinary `collections` accessors — for example
`collections::getOr(resp.headers, "content-type", "")` or
`collections::hasKey(resp.headers, "location")`. There is no dedicated header
function. [[src/builtins/http_package.mfb:__http_parseResponse]]

**Bounded exchange (OS-11).** Each exchange applies a bounded default connect
deadline (30 s) and, on the plaintext path, a per-read deadline (30 s), so a slow
or black-holed peer cannot wedge the calling thread indefinitely — a stalled
exchange fails with a timeout rather than blocking forever. The response is
capped at 64 MiB regardless. [[src/builtins/http_package.mfb:__http_exchangeTcp]]

**No SSRF filtering (OS-10).** The client validates only the URL scheme; it does
**not** restrict which host or address a request may target. A program that
builds a `Url` from untrusted input can therefore be steered at `127.0.0.1`, a
cloud metadata endpoint (`169.254.169.254`), or an RFC-1918/link-local address.
There is no redirect-based amplification (3xx responses are returned as-is, not
followed), but a caller that forwards untrusted URLs must apply its own host
allow/deny policy before calling `http::read`/`http::write`. No default-deny is
imposed because it would break legitimate localhost clients.
[[src/builtins/net_package.mfb:__net_toUrl]]

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

### Server

The `http` package also provides a single-threaded, blocking, user-driven HTTP
server. A program obtains a listener with `http::server(port, host, backlog)`
(plaintext, returning a `net::Listener`) or `http::serverSSL(port, certPath,
keyPath, host, backlog)` (TLS, returning a `tls::TlsListener`), builds an ordered
`List OF http::Route` mapping path patterns to handler functions, and calls
`http::handleRequest(listener, routes)` in its own `DO/LOOP`. Each call accepts
one connection, parses the request, matches its path against the routes in list
order (first match wins), invokes the matched handler
(`FUNC(http::Request) AS http::Response`), writes the response, and closes the
connection. There are no threads, no async, and no keep-alive. [[src/builtins/http_package.mfb:__http_handleRequest]]

`http::route(pattern, handler)` builds a validated `Route`. A pattern is matched
segment by segment: a literal must equal the segment; `:name` captures one
segment into `params["name"]`; a trailing `:name?` is optional; a trailing `*`
captures the whole remaining path into `params["*"]`. `:name?` and `*` are legal
only as final segment(s), and a trailing slash is normalized away (except root
`/`). The matched request exposes `method`, `path` (query stripped,
percent-decoded), `rawPath`, `headers` (lowercased), `query` (from `?a=1&b=2`),
`params` (route captures), `parts` (for `multipart/form-data`), and `body`
(`List OF Byte`) — all read with the ordinary `collections` accessors. [[src/builtins/http_package.mfb:__http_matchPath]]

Handlers build responses with `http::ok`, `http::status`, `http::json`,
`http::responseDefault` (+ `WITH` edits), `http::withHeader`, and `http::bytes`,
and serve files with `http::respondFile` / `http::respondPath` (traversal-safe).
`handleRequest` is crash-proof: a handler failure becomes a `500`, no matching
route becomes a `404`, a malformed request becomes a `400`, an oversize request
(64 MiB cap) becomes a `413`, and a peer reset drops the one connection without
tearing down the server. `Content-Length`, the reason phrase, and
`Connection: close` are supplied on emit. [[src/builtins/http_package.mfb:__http_buildResponse]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | raised by `read`/`write` when the method is empty or contains a space, when a caller header name/value or the URL-derived request-target/Host carries a control byte below `0x20` (in particular CR/LF — request-splitting is rejected, not framed), and by `route` when a `*`/`:name?` segment is not trailing [[src/builtins/http_package.mfb:__http_normalizeMethod]] [[src/builtins/http_package.mfb:__http_hasControlBytes]] |
| `77050003` | `ErrInvalidFormat` | raised by `read`/`write` on a malformed response status line, header block, or chunked framing; the server maps the same class of request-parse failure (malformed request line, non-text headers, bad multipart framing) to a `400` response [[src/builtins/http_package.mfb:__http_parseStatusLine]] |
| `77050010` | `ErrOverflow` | raised by `read`/`write` when the response exceeds the internal 64 MiB size cap; the server maps an oversize request to a `413` response [[src/builtins/http_package.mfb:__http_exchangeTcp]] |

Client transport failures from `net` and `tls` are propagated unchanged; a clean
end of stream terminates an EOF-framed body and is not an error. The server,
by contrast, is crash-proof: it converts request-parse failures, oversize
requests, missing routes, and handler failures into `400`/`413`/`404`/`500`
responses rather than propagating them, and drops a connection on a peer I/O
error without failing the accept loop.
