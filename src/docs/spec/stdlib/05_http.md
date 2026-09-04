# HTTP Client

An HTTP/1.1 client implemented entirely as injected MFBASIC source plus two thin
transport branches. It offers both a blocking form (`read`/`write`) and a
non-blocking, cooperatively-drivable form (`startRead`/`ready`/`pump`/`done`/
`finish`) over a `Stream` resource union — the blocking calls are thin wrappers
over the same non-blocking core. There is no connection pool: each request opens a
connection, sends one request with `Connection: close`, reads until end-of-stream,
parses, and returns; the socket is a scoped resource. All protocol work is string
manipulation; only the transport branches reach native code (`tcp::` for
cleartext, `tls::` for TLS).

`IMPORT http` does not leak `tcp`/`tls`/`net`/`strings`/`collections` into the
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
  body        AS List OF Byte ; body bytes (de-chunked, never the raw frames)
  ok          AS Boolean   ; TRUE iff 200 <= status <= 299
END TYPE
```

`ok` is computed once at parse time from the status code; it is `status >= 200
AND status <= 299`. It does not consider the body or any header.

`body` is `List OF Byte` so binary payloads survive intact; decode it to text
with `toString(resp.body)`. The **same** `Response` record is shared by the
server (see the Server section): the response constructors (`http::ok`,
`http::status`, `http::json`, `http::responseDefault`) build it, and
`http::bytes` / `strings::toBytes` encode a `String` into the body type.

[[src/codegen/builtins/http/mod.rs:Response]]

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

[[src/codegen/builtins/http/mod.rs:__http_parseResponse]]

## Request Construction

The request line is always `METHOD target HTTP/1.1`. The method is validated and
normalized: empty or whitespace-containing methods fail (`77050002`); otherwise
it is uppercased. The request target is the URL path (defaulting to `/` when
empty) with `?query` appended when the URL carries a query.

[[src/codegen/builtins/http/mod.rs:__http_normalizeMethod]] [[src/codegen/builtins/http/mod.rs:__http_requestTarget]]

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

[[src/codegen/builtins/http/mod.rs:__http_buildRequest]] [[src/codegen/builtins/http/mod.rs:__http_isExtraHeader]] [[src/codegen/builtins/http/mod.rs:__http_headerValue]]

### Line terminators

`CRLF` is the two-character `"\r\n"` string literal — the lexer decodes the `\r`
and `\n` escapes — and all framing (status line, header lines, the blank
separator) uses it.

[[src/codegen/builtins/http/mod.rs:__http_buildRequest]]

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

[[src/codegen/builtins/http/mod.rs:__http_parseStatusLine]] [[src/codegen/builtins/http/mod.rs:__http_decToInt]]

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

[[src/codegen/builtins/http/mod.rs:__http_decodeBody]]

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

[[src/codegen/builtins/http/mod.rs:__http_dechunk]] [[src/codegen/builtins/http/mod.rs:__http_hexToInt]]

## Response Size Cap

The accumulated raw response is bounded. `__HTTP_MAX_RESPONSE` is **67108864**
bytes (64 MiB). The cap is checked on the running byte length of the raw stream
after each read; exceeding it fails with `77050010` ("response too large").
The limit applies to the raw, pre-decode stream, so a chunked body's framing
counts against it.

[[src/codegen/builtins/http/mod.rs:__HTTP_MAX_RESPONSE]]

## Transport Selection

The scheme decides the transport. There is no protocol negotiation, redirect
following, or fallback between the two:

```text
exchange(url, request) =
  TLS  branch   if url.scheme = "https"
  TCP  branch   otherwise
```

Both branches connect and write identically aside from the native calls:

- TCP: `tcp::connect(host, port, 30_000)`, then `tcp::write` (String overload).
- TLS: `tls::connect(host, port, 30_000, host)` — 30 s connect deadline, SNI
  server-name = host — then `tls::write`.

The transport is carried as a `Stream` resource union over `tcp::Socket` /
`tls::Socket`; the reader `MATCH`es the active variant and reads 64 KiB at a
time (`tcp::read` / `tls::read`). A read that returns `[]` marks the stream closed
(end of stream). A read that fails with `errorCode::ErrConnectionClosed` is
recovered as a close; any other transport error is captured in the stream's STATE.
The size cap is enforced as bytes accumulate. The socket is a scoped resource
(`RES`), closed once when the `Stream` leaves scope.

[[src/codegen/builtins/http/mod.rs:__http_startExchange]] [[src/codegen/builtins/http/mod.rs:__http_readNet]] [[src/codegen/builtins/http/mod.rs:__http_readTls]]

## Non-Blocking Client

The client is a non-blocking core with a thin blocking veneer. The core is a
`Stream` resource union (`tcp::Socket | tls::Socket`) carrying a `PendingState`
as plan-74 union STATE; five entry points drive an exchange without blocking the
calling thread:

```text
startRead(url, headers, method) -> RES Stream STATE PendingState
  connect per url.scheme, write the whole request, STATE = {sentAll:TRUE, closed:FALSE, raw:[], err:0}
ready(stream)  -> Boolean   ' non-blocking poll(0): would a read return bytes/EOF now?
pump(stream)               ' one readiness-gated 64 KiB read; grows STATE.raw; sets closed/err
done(stream)   -> Boolean   ' STATE.err<>0  OR  STATE.closed  OR  frameComplete(STATE.raw)
finish(stream) -> Response   ' FAIL on STATE.err; else parseResponse(STATE.raw)
```

A program binds the stream, loops `IF ready(s) THEN pump(s)` until `done(s)` —
interleaving its own work between pumps — then calls `finish(s)`. Because plan-80
relocated the union STATE slot to a record offset free in every transport layout,
the STATE works over the `tls::Socket` variant as well as `tcp::Socket`.

## Request Flow

`read`/`write` are thin blocking wrappers over the same core: they `startExchange`,
then loop a blocking readiness wait (`tcp::poll`/`tls::poll` with the 30 s read
deadline — a stalled peer sets `ErrTimeout`) plus `pump` until `done`, then
`finish`.

```text
read(url, headers, method):
  s = startExchange(url, "", hasBody=FALSE, headers, method)
  WHILE done(s)=FALSE: waitReadable(s); pump(s)
  return finish(s)

write(url, body, headers, method):
  s = startExchange(url, body, hasBody=TRUE, headers, method)
  WHILE done(s)=FALSE: waitReadable(s); pump(s)
  return finish(s)
```

`read` sends no body and no `Content-Length`; `write` always sends both. Both
produce the same `Response` a direct read loop would, byte for byte. Neither entry
point follows redirects or retries.

[[src/codegen/builtins/http/mod.rs:__http_read]] [[src/codegen/builtins/http/mod.rs:__http_write]] [[src/codegen/builtins/http/mod.rs:__http_waitReadable]]

## Server

The `http` package also provides a single-threaded, blocking, user-driven HTTP
server — the server-side sibling of the client, in the same package, over the
same `tcp`/`tls` transport, adding no native intrinsics.

### Lifecycle

A program binds a listener and drives its own accept loop:

| Function | Signature |
| --- | --- |
| `http::server` | `server(port AS Integer, host AS String = "0.0.0.0", backlog AS Integer = 128) AS tcp::Listener` |
| `http::serverSSL` | `serverSSL(port AS Integer, certPath AS String, keyPath AS String, host AS String = "0.0.0.0", backlog AS Integer = 128) AS tls::Listener` |
| `http::handleRequest` | `handleRequest(listener AS tcp::Listener, routes AS List OF Route) AS Nothing` — also overloaded for `tls::Listener` |

`http::server` returns the `tcp::Listener` directly (no wrapper resource);
`http::serverSSL` returns a `tls::Listener` owning the bound socket and the
loaded PEM certificate + key, and works on both Linux and macOS.
`http::handleRequest` is overloaded by listener type — both feed one shared
parse/match/dispatch/emit core — and accepts one connection per call. It is
crash-proof: a failing handler becomes a `500` (as does a handler response whose
reason phrase or a header name/value carries a control byte — it is never
serialized, so a reflected `\r\n` cannot split the response), no matching route
a `404`, a malformed request a `400`, an oversize request (64 MiB cap) a `413`,
an over-cap head a `431`, a slow or idle client a `408`, and a peer I/O error
drops the one connection without failing the loop. Every step after the accept
is trapped: a malformed request can cost its own connection, never the process.

Once a client is connected the read is bounded: a connection silent for 10 s
between reads, or whose request is still incomplete 60 s after its first byte,
is answered `408 Request Timeout` and closed. The request head may not exceed
64 KiB, 100 header fields, or an 8 KiB line (`431 Request Header Fields Too
Large`, answered as soon as the excess is seen, without waiting for the rest of
the head); the whole request may not exceed 64 MiB (`413`). The frame is
scanned incrementally, so a large request is examined once, not once per read.

```text
RES s AS tcp::Listener = http::server(8080)
DO
  http::handleRequest(s, routes)
LOOP UNTIL FALSE
```

### Value records

```text
TYPE Request
  method  AS String                        ; uppercased verb
  path    AS String                        ; query stripped, percent-decoded
  rawPath AS String                        ; request-target as received
  headers AS Map OF String TO String       ; field names lowercased; last-wins
  query   AS Map OF String TO String       ; from "?a=1&b=2"; decoded; last-wins
  params  AS Map OF String TO String       ; route captures (:id / :x? / *)
  parts   AS Map OF String TO RequestPart  ; multipart/form-data parts
  body    AS List OF Byte                  ; raw request body bytes
END TYPE

TYPE RequestPart
  filename    AS String        ; "" for a plain field
  contentType AS String        ; "" if absent
  body        AS List OF Byte
END TYPE

TYPE Route
  pattern AS String
  handler AS FUNC(Request) AS Response
END TYPE
```

`Request` fields are public maps read with the ordinary `collections::*`
accessors — there are no `http`-specific request accessors. A path param is
`collections::get(req.params, "id")`; a query value is
`collections::getOr(req.query, "q", "")`; a header is
`collections::getOr(req.headers, "content-type", "")` (keys are lowercased on
parse).

### Routing

Routes are held in an ordered `List OF Route` and tried in list order by
`handleRequest` — **first match wins**. A pattern is matched segment by segment
(split on `/`):

| Segment | Meaning | Binds |
| --- | --- | --- |
| literal | must equal the segment | — |
| `:name` | one non-empty segment | `params["name"]` |
| `:name?` | trailing optional segment | `params["name"]` when present |
| `*` | trailing catch-all (rest of path) | `params["*"]` |

`:name?` and `*` are legal only as final segment(s) — a mid-pattern optional or
wildcard fails `ErrInvalidArgument` at `http::route`. A trailing slash is
normalized away before matching, except the root `/`.

### Request parsing

`handleRequest` reads one full request from the socket and parses it with pure
byte/string code. The request-target is split at the first `?`: the path is
percent-decoded into `Request.path` (via `net::percentDecode`) and the query is
parsed into `Request.query` (via `net::parseQuery`). Header names are lowercased
and OWS-trimmed, last-wins on duplicates — but the header block is parsed
strictly, so a request can never be framed two ways (the request-smuggling
primitives, RFC 9112 §5–6): a second `Content-Length`, a `Content-Length`
alongside `Transfer-Encoding`, whitespace between a field name and its colon, an
obs-fold continuation line, a `Content-Length` that is not an unsigned decimal,
or a `Transfer-Encoding` whose final coding is not exactly `chunked` is a `400`.
The body is exactly the framed bytes — truncated to `Content-Length`, or
de-chunked; a `multipart/form-data` body is split on its boundary into
`Request.parts`. Malformed framing → `400`; exceeding the 64 MiB cap → `413`.
The client-side response parser stays lenient (last-wins, no caps): a sloppy
upstream is not a trust boundary there, and the client always reads to EOF.

### Constructors, combinators, static helpers

| Function | Purpose |
| --- | --- |
| `http::route(pattern, handler) AS Route` | validated route |
| `http::responseDefault() AS Response` | `200` "OK", the `WITH`-edit base |
| `http::ok(body AS String) AS Response` | `200` text/plain |
| `http::status(code, body) AS Response` | arbitrary status, text/plain |
| `http::json(body AS String) AS Response` | `200` application/json |
| `http::withHeader(resp, name, value) AS Response` | copy with one header set |
| `http::bytes(text AS String) AS List OF Byte` | UTF-8 encode into a body |
| `http::respondFile(file AS RES fs::File, contentType AS String = "") AS Response` | serve an open file, closing it |
| `http::respondPath(req, root AS String) AS Response` | serve a request path safely under `root` |

`http::Response` is immutable in place (MFBASIC has no field-target assignment),
so a handler edits it with `WITH`:

```text
MUT resp AS http::Response = http::responseDefault()
resp = WITH resp { status := 418 }
resp = WITH resp { body := http::bytes("I'm a teapot") }
```

`respondPath` is path-traversal-safe: it canonicalizes the requested path and
confines it to `root` (via `fs::isWithin`) *before* opening — any escape (`..`,
absolute, symlink-out) yields `403`, a missing file yields `404`, never a read
outside `root`. `Content-Length`, the reason phrase, and `Connection: close`
are always server-supplied on emit; a handler-set `Content-Length` is ignored.

[[src/codegen/builtins/http/mod.rs:__http_handleRequest]] [[src/codegen/builtins/http/mod.rs:__http_matchPath]]

## See Also

* ./mfb man http — the per-function API reference
* ./mfb spec stdlib url — the `net::Url` model that drives target/host/scheme
* ./mfb spec stdlib transports — the `tcp`/`tls` model this client and server run on
* ./mfb spec architecture frontend — how this source package is injected
* ./mfb spec unicode strings-model — byte vs grapheme length (Content-Length)
* ./mfb spec memory arenas — where copyable `Response` values live
