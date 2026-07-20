# read

Perform one blocking, body-less HTTP/1.1 request and return the response.

## Synopsis

```
http::read(url AS net::Url) AS Response
http::read(url AS net::Url, headers AS Map OF String TO String) AS Response
http::read(url AS net::Url, headers AS Map OF String TO String, method AS String) AS Response
```

## Package

`http`

## Imports

```
IMPORT net
IMPORT http
```

`IMPORT net` is required because the first argument is a `net::Url` (build one
with `net::toUrl`). The `Response` type is provided by `http`.

## Description

`read` performs exactly one blocking HTTP/1.1 request that carries **no body**
and returns the reply as an `http::Response` value. It opens a fresh connection
to `url.host` on `url.port` — plaintext through the `net` package for an `http://`
URL, TLS through the `tls` package for an `https://` URL — writes the request,
reads the response to end of stream, closes the connection, and returns. The
connection is never reused; every call sends `Connection: close`.
[[src/builtins/http_package.mfb:__http_exchangeTcp]] [[src/builtins/http_package.mfb:__http_buildRequest]]

The `method` argument defaults to `GET` and may be any body-less verb (`HEAD`,
`DELETE`, `OPTIONS`, and so on). It is uppercased before it is sent, so `"get"`
and `"GET"` are equivalent. [[src/builtins/http.rs:default_argument_padding]] [[src/builtins/http_package.mfb:__http_normalizeMethod]]

The optional `headers` map contributes request headers. A caller entry whose name
matches one of the automatic headers — `Host`, `User-Agent`, or `Accept` — replaces
that default (the match is case-insensitive); any other entry is appended verbatim.
The framing headers `Connection` and `Content-Length` are reserved: `Connection`
is always `close` and cannot be overridden, and no body means no `Content-Length`
is sent. Every header name and value, along with the request target and `Host`
derived from the URL, is rejected if it contains a control byte (any byte below
`0x20`, such as CR or LF), so a caller cannot smuggle extra headers or a second
request line. [[src/builtins/http_package.mfb:__http_hasControlBytes]] [[src/builtins/http_package.mfb:__http_isExtraHeader]]

The request target is `url.path` (an empty path is normalized to `/`) followed by
`?` and `url.query` when a query is present; the URL fragment is never sent.
[[src/builtins/http_package.mfb:__http_requestTarget]]

The returned `Response` exposes `status` (Integer), `reason` (String, `""` when
omitted), `httpVersion` (String, e.g. `"1.1"`), `headers` (a `Map OF String TO
String`), `body` (a `List OF Byte`), and `ok` (Boolean, `TRUE` only when `status`
is in `200..299`). Header field names in `headers` are lowercased and duplicates
collapse last-wins, so read a header with the ordinary collections accessors, e.g.
`collections::getOr(resp.headers, "content-type", "")`. Redirects are **not**
followed: a 3xx reply is returned as-is, with `ok` `FALSE` and its target in
`resp.headers` under `"location"`. A `chunked` transfer-encoded body is de-chunked
before it is placed in `body`. [[src/builtins/http_package.mfb:__http_parseResponse]] [[src/builtins/http_package.mfb:__http_decodeBody]]

The client applies a 30-second connect deadline and, for plaintext, a 30-second
per-read deadline so a stalled or black-holed peer fails cleanly rather than
wedging the calling thread; the 64 MiB response cap bounds memory for a peer that
streams without end. [[src/builtins/http_package.mfb:__HTTP_CONNECT_TIMEOUT_MS]]

## Overloads

**`http::read(url AS net::Url) AS Response`**

Sends a `GET` with no caller headers. [[src/builtins/http.rs:resolve_call]]

**`http::read(url AS net::Url, headers AS Map OF String TO String) AS Response`**

Sends a `GET` with the supplied headers.

**`http::read(url AS net::Url, headers AS Map OF String TO String, method AS String) AS Response`**

Sends `method` (uppercased) with the supplied headers. This is the full form; the
shorter overloads default `headers` to an empty map and `method` to `GET`.
[[src/builtins/http.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `url` | `net::Url` | The target URL. `url.scheme` selects transport (`https` → TLS on default port 443, otherwise plaintext on default port 80); `url.host`, `url.port`, `url.path`, and `url.query` form the connection and request target. |
| `headers` | `Map OF String TO String` | Optional request headers. Names matching `Host`/`User-Agent`/`Accept` override the defaults case-insensitively; others are appended. No name or value may contain a control byte. Defaults to an empty map. |
| `method` | `String` | Optional request method; uppercased before sending. Must be non-empty and contain no space. Defaults to `GET`. |

## Return value

| Type | Description |
| --- | --- |
| `Response` | The parsed reply: `status`, `reason`, `httpVersion`, `headers` (lowercased field names), `body` (raw bytes, de-chunked when the reply was `chunked`), and `ok` (`TRUE` only for a 2xx status). A 3xx redirect is returned with `ok` `FALSE`, not followed. [[src/builtins/http_package.mfb:__http_parseResponse]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `method` is empty or contains a space, or a caller header name/value or the URL-derived request target/`Host` contains a control byte (below `0x20`). [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] [[src/builtins/http_package.mfb:__http_normalizeMethod]] |
| `77050003` | `ErrInvalidFormat` | The response status line, header block, or `chunked` framing (chunk-size field, chunk length, or terminator) is malformed. [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] [[src/builtins/http_package.mfb:__http_parseStatusLine]] |
| `77050010` | `ErrOverflow` | The accumulated response exceeds the internal 64 MiB size cap. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] [[src/builtins/http_package.mfb:__HTTP_MAX_RESPONSE]] |

Connect, DNS, read, write, timeout, and TLS failures are not raised by `read`
itself: they propagate unchanged from the underlying `net` and `tls` calls (for
example `ErrAddressNotFound`, `ErrNetworkFailed`, `ErrReadTimeout`, or
`ErrTlsFailed`). [[src/builtins/http_package.mfb:__http_exchangeTls]]

## Examples

A plain GET, reading the status line:

```
IMPORT net
IMPORT http
IMPORT io

SUB main()
  LET r = http::read(net::toUrl("http://example.com/"))
  io::print(toString(r.status) & " " & r.reason)
END SUB
```

A GET with an Authorization header and an explicit method, then a header lookup:

```
IMPORT net
IMPORT http
IMPORT collections
IMPORT io

SUB main()
  LET h = Map OF String TO String { "Authorization" := "Bearer xyz" }
  LET r = http::read(net::toUrl("http://example.com/item/1"), h, "DELETE")
  LET ct = collections::getOr(r.headers, "content-type", "")
  io::print(ct)
END SUB
```

## See also

- `mfb man http write`
- `mfb man net toUrl`
- `mfb man collections getOr`
