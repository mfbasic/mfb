# handleRequest

Accept one connection, serve it against an ordered route list, and close it.

## Synopsis

```
http::handleRequest(listener AS net::Listener, routes AS List OF Route) AS Nothing
http::handleRequest(listener AS tls::TlsListener, routes AS List OF Route) AS Nothing
```

## Package

`http`

## Imports

```
IMPORT http
IMPORT net
```

`IMPORT net` is required to name the `net::Listener` type of the binding that
`http::server` returns. For the TLS overload use `IMPORT tls` instead, to name the
`tls::TlsListener` returned by `http::serverSSL`.

## Description

`handleRequest` performs exactly **one** connection's worth of work: it accepts a
single inbound connection from `listener`, reads one HTTP/1.1 request, matches it
against `routes`, invokes the matched handler, writes the resulting
`http::Response`, and closes the accepted socket. It is meant to be driven from a
user-owned `DO`/`LOOP`. The listener itself is **borrowed** — it stays open across
calls and is closed only by its own lexical drop (or `net::close` / `tls::close`).
The accepted socket is owned by the call and closed by lexical drop on return.
[[src/builtins/http_package.mfb:__http_handleRequest]]

The server is single-threaded and blocking: the accept call blocks until a client
arrives, and one request is served at a time in the caller's loop. No timeout is
passed to the underlying `net::accept` / `tls::accept`, so the wait is unbounded.
[[src/builtins/net.rs:ACCEPT]] [[src/builtins/tls.rs:ACCEPT]]

**Reading.** Bytes are read in 64 KiB chunks and appended to a raw byte buffer
until the frame is complete — the header terminator `\r\n\r\n` has been seen and
the body implied by `Content-Length` has arrived, or, for
`Transfer-Encoding: chunked`, the `0\r\n\r\n` terminator has arrived. A read that
fails is treated as end of stream. If the peer closes before sending anything
(zero bytes read), the call returns without writing a response.
[[src/builtins/http_package.mfb:__http_frameComplete]]

**Size cap.** The accumulated request may not exceed **67108864** bytes (64 MiB).
Once the buffer passes that size, reading stops and the connection is answered
with a `413 Payload Too Large`.
[[src/builtins/http_package.mfb:__HTTP_MAX_REQUEST]]

**Parsing.** The request line yields an uppercased `method` and a request target.
The target is split at the first `?`: the part before it is percent-decoded into
`Request.path` (falling back to the raw text if decoding fails), the part after it
is parsed into `Request.query`; `Request.rawPath` keeps the target as received.
Header field names are lowercased and duplicates collapse last-wins. A chunked
body is de-chunked, and a `multipart/form-data` body is split into
`Request.parts` keyed by each part's `name`. `Request.body` holds the raw body
bytes.
[[src/builtins/http_package.mfb:__http_parseRequest]]

**Matching.** Routes are tested in list order and the **first** match wins. Path
matching is segment-based on the decoded path with a single trailing `/` ignored;
`:name` binds one required segment, `:name?` binds an optional trailing segment,
and `*` binds all remaining segments joined by `/`. Bound captures are placed in
`Request.params` (the wildcard under the key `"*"`) before the handler runs.
[[src/builtins/http_package.mfb:__http_matchPath]]

**Crash-proofing.** The accept loop never dies on a bad client. A handler that
fails for any reason is answered with a built-in `500 Internal Server Error`; a
path matching no route is answered with `404 Not Found`; an unparsable request
line or header block is answered with `400 Bad Request`; an over-cap request is
answered with `413 Payload Too Large`. A write that fails mid-response drops the
connection and returns normally.
[[src/builtins/http_package.mfb:__http_buildResponse]] [[src/builtins/http_package.mfb:__http_invokeHandler]]

**Emission.** The status line is `HTTP/1.1 <status> <reason>`; an empty
`Response.reason` is filled in from a built-in table keyed by status code, falling
back to `OK` below 300, `Redirect` below 400, `Client Error` below 500, and
`Server Error` otherwise. Handler-set `Content-Length` and `Connection` headers
are dropped so framing stays correct, and the server always emits its own
`Content-Length` (the byte length of `Response.body`) plus `Connection: close`.
The body is written only when it is non-empty.
[[src/builtins/http_package.mfb:__http_serializeHead]] [[src/builtins/http_package.mfb:__http_reasonPhrase]]

## Overloads

**`http::handleRequest(listener AS net::Listener, routes AS List OF Route) AS Nothing`**

Plaintext HTTP over the native `net` transport. Selected when the first argument
is a `net::Listener`, as returned by `http::server`.

**`http::handleRequest(listener AS tls::TlsListener, routes AS List OF Route) AS Nothing`**

HTTPS: `tls::accept` performs the server-side TLS handshake, then the identical
parse/match/dispatch/emit core runs over the encrypted socket. Selected when the
first argument is a `tls::TlsListener`, as returned by `http::serverSSL`. The
overload is resolved from the first argument's type at IR lowering; route lists
and handlers are interchangeable between the two.
[[src/builtins/http.rs:implementation_name]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `listener` | `net::Listener` or `tls::TlsListener` | An open listening socket to accept one connection from. Borrowed — it remains open and usable after the call. Must be bound with `RES`. |
| `routes` | `List OF Route` | Routes tested in list order, first match wins. Build entries with `http::route`. An empty list makes every request a `404`. |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `handleRequest` is a `SUB` and yields no value; its effect is the served connection. [[src/builtins/http.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `listener` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070003` | `ErrNetworkFailed` | The `accept` call on `listener` fails for a reason other than an interrupting signal (`EINTR` is retried). [[src/target/shared/code/net/io.rs:lower_net_accept_helper]] |
| `77070008` | `ErrTlsFailed` | TLS overload only: the server-side handshake, or the per-connection TLS setup, fails. [[src/target/shared/code/tls/mod.rs:lower_tls_accept_helper]] |
| `77010001` | `ErrOutOfMemory` | An arena allocation for the accepted socket handle, the request buffer, or the response text fails. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

Malformed, oversized, unmatched, and handler-failing requests are **not** errors:
they become `400`, `413`, `404`, and `500` responses. Read and write failures on
the accepted connection are absorbed, and the call returns normally.

## Examples

A plaintext accept loop with one route:

```
IMPORT http
IMPORT net
IMPORT collections

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("hello from " & req.path)
END FUNC

SUB main()
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/", home))
  RES s AS net::Listener = http::server(8080)
  DO
    http::handleRequest(s, routes)
  LOOP UNTIL FALSE
END SUB
```

Reading a captured path parameter, and serving the same routes over TLS:

```
IMPORT http
IMPORT tls
IMPORT collections

FUNC showUser(req AS http::Request) AS http::Response
  RETURN http::ok("user " & collections::getOr(req.params, "id", ""))
END FUNC

SUB secureMain()
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/user/:id", showUser))
  RES s AS tls::TlsListener = http::serverSSL(8443, "cert.pem", "key.pem")
  DO
    http::handleRequest(s, routes)
  LOOP UNTIL FALSE
END SUB
```

## See also

- `mfb man http server`
- `mfb man http serverSSL`
- `mfb man http route`
- `mfb man http ok`
- `mfb man http status`
- `mfb man http respondPath`
