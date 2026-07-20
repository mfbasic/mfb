# write

Perform one blocking HTTP/1.1 request that carries a body and return the response.

## Synopsis

```
http::write(url AS net::Url, body AS String) AS Response
http::write(url AS net::Url, body AS String, headers AS Map OF String TO String) AS Response
http::write(url AS net::Url, body AS String, headers AS Map OF String TO String, method AS String) AS Response
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

`write` performs exactly one blocking HTTP/1.1 request that carries a **body**
and returns the reply as an `http::Response` value. It opens a fresh connection
to `url.host` on `url.port` — plaintext through the `net` package for an `http://`
URL, TLS through the `tls` package for an `https://` URL — writes the request
line, headers, and body, reads the response to end of stream, closes the
connection, and returns. The connection is never reused; every call sends
`Connection: close`. [[src/builtins/http_package.mfb:__http_write]] [[src/builtins/http_package.mfb:__http_buildRequest]]

The `body` is sent verbatim as UTF-8 bytes. A `Content-Length` header equal to
the body's **byte** length is always generated, so a caller cannot override the
framing. [[src/builtins/http_package.mfb:__http_buildRequest]]

The `method` argument defaults to `POST` and may be any body-carrying verb
(`PUT`, `PATCH`, and so on). It is uppercased before it is sent, so `"put"` and
`"PUT"` are equivalent. [[src/builtins/http.rs:default_argument_padding]] [[src/builtins/http_package.mfb:__http_normalizeMethod]]

The optional `headers` map contributes request headers. A caller entry whose name
matches one of the automatic headers — `Host`, `User-Agent`, or `Accept` — replaces
that default (the match is case-insensitive); any other entry is appended verbatim.
The framing headers `Connection` and `Content-Length` are reserved: `Connection`
is always `close` and cannot be overridden, and `Content-Length` is always derived
from the body — a caller entry for either is dropped. Every header name and value,
along with the request target and `Host` derived from the URL, is rejected if it
contains a control byte (any byte below `0x20`, such as CR or LF), so a caller
cannot smuggle extra headers or a second request line.
[[src/builtins/http_package.mfb:__http_isExtraHeader]] [[src/builtins/http_package.mfb:__http_hasControlBytes]]

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
before it is placed in `body`. [[src/builtins/http_package.mfb:__http_parseResponse]]

The client applies a 30-second connect deadline and, for plaintext, a 30-second
per-read deadline so a stalled or black-holed peer fails cleanly rather than
wedging the calling thread; the 64 MiB response cap bounds memory for a peer that
streams without end. [[src/builtins/http_package.mfb:__HTTP_MAX_RESPONSE]]

## Overloads

**`http::write(url AS net::Url, body AS String) AS Response`**

Sends `body` with the default `POST` method and no caller headers.
[[src/builtins/http.rs:resolve_call]]

**`http::write(url AS net::Url, body AS String, headers AS Map OF String TO String) AS Response`**

Sends `body` with the supplied headers, still using the default `POST` method.

**`http::write(url AS net::Url, body AS String, headers AS Map OF String TO String, method AS String) AS Response`**

Sends `body` with the supplied headers using an explicit body-carrying method
(uppercased). This is the full form; the shorter overloads default `headers` to
an empty map and `method` to `POST`. [[src/builtins/http.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `url` | `net::Url` | The target URL. `url.scheme` selects transport (`https` → TLS on default port 443, otherwise plaintext on default port 80); `url.host`, `url.port`, `url.path`, and `url.query` form the connection and request target. |
| `body` | `String` | The request payload, sent verbatim as UTF-8 bytes. Its byte length becomes the generated `Content-Length` header. |
| `headers` | `Map OF String TO String` | Optional request headers. Names matching `Host`/`User-Agent`/`Accept` override the defaults case-insensitively; others are appended. `Content-Length` and `Connection` entries are dropped (both are forced). No name or value may contain a control byte. Defaults to an empty map. |
| `method` | `String` | Optional request method; uppercased before sending. Must be non-empty and contain no space. Defaults to `POST`. |

## Return value

| Type | Description |
| --- | --- |
| `Response` | The parsed reply: `status`, `reason`, `httpVersion`, `headers` (lowercased field names), `body` (raw bytes, de-chunked when the reply was `chunked`), and `ok` (`TRUE` only for a 2xx status). A 3xx redirect is returned with `ok` `FALSE`, not followed. [[src/builtins/http_package.mfb:Response]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `method` is empty or contains a space, or a caller header name/value or the URL-derived request target/`Host` contains a control byte (below `0x20`). [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] [[src/builtins/http_package.mfb:__http_normalizeMethod]] |
| `77050003` | `ErrInvalidFormat` | The response status line, header block, or `chunked` framing (chunk-size field, chunk length, or terminator) is malformed. [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] [[src/builtins/http_package.mfb:__http_decToInt]] |
| `77050010` | `ErrOverflow` | The accumulated response exceeds the internal 64 MiB size cap. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] [[src/builtins/http_package.mfb:__HTTP_MAX_RESPONSE]] |

Connect, DNS, read, write, timeout, and TLS failures are not raised by `write`
itself: they propagate unchanged from the underlying `net` and `tls` calls (for
example `ErrAddressNotFound`, `ErrNetworkFailed`, `ErrReadTimeout`, or
`ErrTlsFailed`). [[src/builtins/http_package.mfb:__http_exchange]]

## Examples

POST a JSON body with an explicit content type, then read the status:

```
IMPORT net
IMPORT http
IMPORT io

SUB main()
  LET ct = Map OF String TO String { "Content-Type" := "application/json" }
  LET r = http::write(net::toUrl("http://example.com/items"), "{\"name\":\"a\"}", ct)
  io::print(toString(r.status))
END SUB
```

PUT a body with an explicit method, then check success:

```
IMPORT net
IMPORT http
IMPORT io

SUB main()
  LET headers AS Map OF String TO String = Map OF String TO String {}
  LET r = http::write(net::toUrl("http://example.com/item/1"), "updated", headers, "PUT")
  IF r.ok THEN
    io::print("saved")
  END IF
END SUB
```

## See also

- `mfb man http read`
- `mfb man net toUrl`
- `mfb man collections getOr`
