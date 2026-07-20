# types

the http package record types

## Synopsis

```
http::Response
http::Request
http::RequestPart
http::Route
```

## Package

http

## Imports

```
IMPORT http
```

`http` is a built-in package, so `IMPORT http` needs no manifest dependency. Its
types are declared by the package source; either spelling resolves once `http` is
imported, but the conventional one is package-qualified (`http::Response`), which
is the form used throughout this manual. A program that also builds URLs for the
client adds `IMPORT net` for `net::Url`. [[src/builtins/http.rs:augmented_project]]

## Description

The `http` package defines four record types. All four are ordinary copyable
value records: none holds a socket or any other resource handle, so a `Response`
or a `Request` can be assigned, copied, stored in a collection, returned from a
function, and sent across threads even though the exchange that produced it has
already closed its connection. [[src/builtins/http_package.mfb:Response]]

`Response` is shared by both halves of the package. On the client it is what
`http::read` and `http::write` return; on the server it is what a route handler
returns and what `http::handleRequest` writes to the wire. Handlers rarely build
one field by field — `http::ok`, `http::status`, `http::json`, `http::bytes`,
`http::withHeader`, and `http::responseDefault` (plus `WITH` edits) construct and
amend it. The framing headers `Content-Length` and `Connection` are supplied by
the package on emit, so a handler does not set them. [[src/builtins/http_package.mfb:__http_buildResponse]]

`Request` is the server-side view of one parsed inbound request, passed to the
matched handler. Its four map fields — `headers`, `query`, `params`, and `parts` —
are ordinary `Map` values read with the `collections` accessors, for example
`collections::getOr(req.query, "page", "1")` or
`collections::hasKey(req.params, "id")`. There is no dedicated header or
parameter function. [[src/builtins/http_package.mfb:__http_matchPath]]

`RequestPart` appears only inside `Request.parts`, which is populated when the
request body is `multipart/form-data`. Each part is keyed by its
Content-Disposition `name`, and a part is distinguished as an uploaded file
rather than a plain form field by having a non-empty
`filename`. [[src/builtins/http_package.mfb:RequestPart]]

`Route` binds one path pattern to one handler function. A program builds a
validated `Route` with `http::route(pattern, handler)` — which rejects a malformed
pattern up front — collects them into an ordered `List OF http::Route`, and passes
that list to `http::handleRequest`, which matches in list order, first match wins.
Building a `Route` by hand with a record literal skips that
validation. [[src/builtins/http_package.mfb:__http_handleRequest]]

## Types

### http::Response

One HTTP response: status line, headers, and body. Returned by the client calls and by every route handler. [[src/builtins/http_package.mfb:Response]]

| Field | Type | Description |
| --- | --- | --- |
| `status` | `Integer` | The HTTP status code, e.g. `200`, `404`, `500`. |
| `reason` | `String` | The reason phrase, e.g. `"OK"`; `""` when the peer omitted it. The server supplies one on emit. |
| `httpVersion` | `String` | The version from the status line, `"1.0"` or `"1.1"`. |
| `headers` | `Map OF String TO String` | Response header fields. Names are lowercased (HTTP field names are case-insensitive) and duplicates collapse last-wins. |
| `body` | `List OF Byte` | The raw body bytes, de-chunked when `Transfer-Encoding` was chunked, so binary payloads survive intact; decode text with `toString(resp.body)`. Empty for a `204` or `304`. |
| `ok` | `Boolean` | `TRUE` exactly when `status` is in `200 .. 299`. A redirect is *not* ok — 3xx responses are returned as-is, never followed. |

### http::Request

One parsed inbound HTTP request, handed to the matched route handler. [[src/builtins/http_package.mfb:Request]]

| Field | Type | Description |
| --- | --- | --- |
| `method` | `String` | The request method, uppercased, e.g. `"GET"`, `"POST"`. |
| `path` | `String` | The request path with the query string stripped and percent-escapes decoded, e.g. `"/test/42"`. Match on this. |
| `rawPath` | `String` | The request-target exactly as received, undecoded and including any query string. |
| `headers` | `Map OF String TO String` | Request header fields, names lowercased, duplicates last-wins. |
| `query` | `Map OF String TO String` | Query-string parameters from `?a=1&b=2`, percent-decoded, duplicates last-wins. |
| `params` | `Map OF String TO String` | Captures bound by the matched route pattern: `:name` captures one segment under `"name"`, and a trailing `*` captures the remaining path under `"*"`. Empty when the pattern is all literals. |
| `parts` | `Map OF String TO RequestPart` | The `multipart/form-data` parts, keyed by each part's Content-Disposition name. Empty for a request that is not multipart. |
| `body` | `List OF Byte` | The raw request body bytes, capped at 64 MiB (an oversize request becomes a `413` and never reaches a handler). |

### http::RequestPart

One part of a `multipart/form-data` request body, as found in `Request.parts`. [[src/builtins/http_package.mfb:RequestPart]]

| Field | Type | Description |
| --- | --- | --- |
| `filename` | `String` | The filename from the part's Content-Disposition; `""` for a plain form field. Non-empty marks the part as an uploaded file. Treat it as untrusted input — never join it onto a path unchecked. |
| `contentType` | `String` | The part's own `Content-Type`; `""` when the part declared none. |
| `body` | `List OF Byte` | The part's raw bytes, verbatim; decode text with `toString(part.body)`. |

### http::Route

One entry in a server's routing table: a path pattern and the handler to invoke on a match. Built with `http::route`. [[src/builtins/http_package.mfb:Route]]

| Field | Type | Description |
| --- | --- | --- |
| `pattern` | `String` | The path pattern, matched segment by segment: a literal segment must match exactly, `:name` captures one segment, a trailing `:name?` is an optional segment, and a trailing `*` captures the whole remaining path. `:name?` and `*` are legal only as final segments; a trailing slash is normalized away except for root `/`. |
| `handler` | `FUNC(Request) AS Response` | The function invoked when `pattern` matches; it receives the parsed `http::Request` and returns the `http::Response` to send. A handler that fails becomes a `500` rather than tearing down the server. |

## See also

- `mfb man http`
- `mfb man http read`
- `mfb man http route`
- `mfb man http handleRequest`
- `mfb man http ok`
- `mfb man net types` — `net::Url`, the client's request target
