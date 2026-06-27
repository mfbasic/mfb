# HTTP Client

A blocking HTTP/1.1 client implemented entirely as injected MFBASIC source plus
two thin transport branches. There is no socket state, connection pool, or
asynchronous machinery in the package itself: each request opens a connection,
sends one request with `Connection: close`, reads until end-of-stream, parses,
and returns. All protocol work is string manipulation; only the transport
branches reach native code (`net::` for cleartext, `tls::` for TLS).

`IMPORT http` does not leak `net`/`tls`/`strings`/`collections` into the
importing program — the package's own imports are file-scoped.

## Response Record

A request yields a plain, copyable value record. There is no separate header
type: `headers` is a standard `Map`.

```text
TYPE Response
  status      AS Integer   ; numeric status code, e.g. 200, 404
  reason      AS String    ; reason phrase, "" when omitted from the status line
  httpVersion AS String    ; "1.0" / "1.1" — the token after "HTTP/"
  headers     AS Map OF String TO String   ; lowercased field name -> value
  body        AS String    ; decoded body (de-chunked, never the raw frames)
  ok          AS Boolean   ; TRUE iff 200 <= status <= 299
END TYPE
```

`ok` is computed once at parse time from the status code; it is `status >= 200
AND status <= 299`. It does not consider the body or any header.

[[src/builtins/http_package.mfb:Response]]

## Header Model

Response field names are case-insensitive on the wire, so the parser normalizes
them: each header line is split at the first `:`, the name is trimmed and
lowercased, and the value is trimmed. A program therefore reads a header with
the ordinary collection accessors against a lowercased key:

```text
collections::getOr(resp.headers, "content-type", "")
```

Duplicate field names **collapse last-wins**: the parser writes each field into
the map with `collections::set`, so a later occurrence overwrites an earlier one.
There is no comma-joining of duplicate values.

[[src/builtins/http_package.mfb:__http_parseResponse]]

## Request Construction

The request line is always `METHOD target HTTP/1.1`. The method is validated and
normalized: empty or whitespace-containing methods fail (`77050002`); otherwise
it is uppercased. The request target is the URL path (defaulting to `/` when
empty) with `?query` appended when the URL carries a query.

[[src/builtins/http_package.mfb:__http_normalizeMethod]] [[src/builtins/http_package.mfb:__http_requestTarget]]

Four headers are emitted automatically, each overridable by a caller header of
the same name (matched case-insensitively):

| Header           | Default                                    | Overridable | Notes |
|------------------|--------------------------------------------|-------------|-------|
| `Host`           | `url.host`, or `host:port` for a non-default port | yes  | default port is 443 for `https`, else 80 |
| `User-Agent`     | `mfb-http/1`                               | yes         | |
| `Accept`         | `*/*`                                      | yes         | |
| `Connection`     | `close`                                    | no (forced) | always single-shot |
| `Content-Length` | byte length of the body                    | no (forced) | only when a body is sent |

A caller-supplied header is treated as "extra" and appended verbatim only when
its lowercased name is none of `host`, `user-agent`, `accept`, `connection`,
`content-length`. The first three are folded into the automatic overrides above;
`connection` and `content-length` are reserved framing headers that the caller
cannot override.

The body, when present, follows the blank `CRLF` line; `Content-Length` is the
body's **byte** length (`strings::byteLen`), not its grapheme count.

[[src/builtins/http_package.mfb:__http_buildRequest]] [[src/builtins/http_package.mfb:__http_isExtraHeader]] [[src/builtins/http_package.mfb:__http_headerValue]]

### Line terminators

This file is lexed in internal mode, where the `\r` string escape is not decoded.
`CRLF` is therefore constructed from raw bytes 13 and 10 rather than a literal,
and all framing (status line, header lines, the blank separator) uses it.

[[src/builtins/http_package.mfb:__http_crlf]]

## Response Parsing

The raw byte stream is split at the first `CRLF CRLF` into a head section and a
body section. If no blank-line separator is present, the whole stream is treated
as the head and the body is empty.

```text
raw            = head-section  CRLF CRLF  body-section
head-section   = status-line  *( CRLF header-line )
status-line    = "HTTP/" version SP status [ SP reason ]
header-line    = field-name ":" field-value
```

Status-line parsing: the token before the first space must start with `HTTP/`;
the remaining prefix after `HTTP/` becomes `httpVersion`. The text after the
first space is split at its first space into the numeric status and the reason
phrase (reason is `""` when there is no second space). Status digits are parsed
in base 10; any non-digit fails (`77050003`).

[[src/builtins/http_package.mfb:__http_parseStatusLine]] [[src/builtins/http_package.mfb:__http_decToInt]]

## Body Decoding

```text
body =
  ""                              if status is 204 or 304
  dechunk(body-section)           if Transfer-Encoding contains "chunked"
  body-section                    otherwise
```

`204 No Content` and `304 Not Modified` always yield an empty body regardless of
what was read. The `transfer-encoding` header (already lowercased) is matched
case-insensitively for the substring `chunked`.

[[src/builtins/http_package.mfb:__http_decodeBody]]

### Chunked transfer decoding

A `chunked` body is de-chunked into the plain bytes. Each chunk is a hex size
line, optionally with a `;`-delimited chunk extension that is ignored, followed
by `CRLF`, that many data bytes, and a trailing `CRLF`. A zero-size chunk
terminates the body; trailers after it are discarded. Malformed framing
(missing terminator, bad hex, or data running past the buffer) fails with
`77050003`.

```text
chunked-body = *chunk  last-chunk
chunk        = HEX [ ";" ext ] CRLF  data  CRLF
last-chunk   = "0" CRLF
```

[[src/builtins/http_package.mfb:__http_dechunk]] [[src/builtins/http_package.mfb:__http_hexToInt]]

## Response Size Cap

The accumulated raw response is bounded. `__HTTP_MAX_RESPONSE` is **67108864**
bytes (64 MiB). The cap is checked on the running byte length of the raw stream
after each read; exceeding it fails with `77050010` ("response too large").
The limit applies to the raw, pre-decode stream, so a chunked body's framing
counts against it.

[[src/builtins/http_package.mfb:__HTTP_MAX_RESPONSE]]

## Transport Selection

The scheme decides the transport. There is no protocol negotiation, redirect
following, or fallback between the two:

```text
exchange(url, request) =
  TLS  branch   if url.scheme = "https"
  TCP  branch   otherwise
```

Both branches are structurally identical aside from the native calls:

- TCP: `net::connectTcp(host, port)`, then `net::writeText`, then a read loop of
  `net::readText(sock, 65536)`.
- TLS: `tls::connect(host, port, 0, host)` — timeout 0, SNI server-name = host —
  then `tls::writeText`, then `tls::readText(sock, 65536)`.

Each loop reads 64 KiB at a time and concatenates. A read that returns `""` ends
the loop (end of stream). A read that fails with `errorCode::ErrConnectionClosed`
is recovered as `""` (treated as a clean close, ending the loop); any other
transport error propagates. The size cap is enforced inside the loop. The socket
is a scoped resource (`RES`), closed when the exchange function returns.

[[src/builtins/http_package.mfb:__http_exchange]] [[src/builtins/http_package.mfb:__http_exchangeTcp]] [[src/builtins/http_package.mfb:__http_exchangeTls]]

## Request Flow

```text
read(url, headers, method):
  verb    = normalizeMethod(method)
  request = buildRequest(verb, url, "", hasBody=FALSE, headers)
  raw     = exchange(url, request)
  return    parseResponse(raw)

write(url, body, headers, method):
  verb    = normalizeMethod(method)
  request = buildRequest(verb, url, body, hasBody=TRUE, headers)
  raw     = exchange(url, request)
  return    parseResponse(raw)
```

`read` sends no body and no `Content-Length`; `write` always sends both. Neither
entry point follows redirects or retries.

[[src/builtins/http_package.mfb:__http_read]] [[src/builtins/http_package.mfb:__http_write]]

## See Also

* ./mfb man http — the per-function API reference
* ./mfb spec stdlib url — the `net::Url` model that drives target/host/scheme
* ./mfb spec architecture frontend — how this source package is injected
* ./mfb spec unicode strings-model — byte vs grapheme length (Content-Length)
* ./mfb spec memory arenas — where copyable `Response` values live
