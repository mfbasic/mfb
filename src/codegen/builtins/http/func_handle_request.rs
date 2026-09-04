//! `http::handleRequest` — descriptor entry (source-backed). Overloaded by listener
//! type: a `tcp::Listener` rewrites to `__http_handleRequest`, a `tls::Listener`
//! to `__http_handleRequestSSL` — two `Implementation`s the generic overload
//! resolution selects by the first argument's type (the datetime/net idiom, no
//! custom resolver).

use crate::codegen::registry::{
    Body, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Accept one connection, serve it against an ordered route list, and close it."#;

const DESC: &str = r#"`handleRequest` performs exactly **one** connection's worth of work: it accepts a
single inbound connection from `listener`, reads one HTTP/1.1 request, matches it
against `routes`, invokes the matched handler, writes the resulting
`http::Response`, and closes the accepted socket. It is meant to be driven from a
your own `DO`/`LOOP`. The listener stays open across
calls and is closed only when its own binding goes out of scope (or by
`tcp::close` / `tls::close`). The accepted socket belongs to the call and is
closed when it returns.

The server is single-threaded and blocking: the accept call blocks until a client
arrives, and one request is served at a time in the caller's loop. No timeout is
passed to the underlying `tcp::accept` / `tls::accept`, so the wait for a client
is unbounded; once a client is connected, the read is bounded (below).

**Reading.** Bytes are read in 64 KiB chunks and appended to a raw byte buffer
until the frame is complete — the header terminator `\r\n\r\n` has been seen and
the body implied by `Content-Length` has arrived, or, for
`Transfer-Encoding: chunked`, the terminating zero-length chunk has arrived. The
frame is scanned incrementally, so a large request costs one pass. A read that
fails is treated as end of stream. If the peer closes before sending anything
(zero bytes read), the call returns without writing a response; a peer that
closes part-way through a request is answered `400 Bad Request`.

**Deadlines.** A connection that sends nothing for **10 seconds** between reads,
or whose request is still incomplete **60 seconds** after its first byte, is
answered `408 Request Timeout` (silently closed if not one byte arrived) so a
slow or idle client cannot hold the accept loop.

**Size caps.** The request head (request line plus header block) may not exceed
**65536** bytes (64 KiB), hold more than **100** header fields, or contain a line
longer than **8192** bytes; exceeding any of these is answered
`431 Request Header Fields Too Large` as soon as it is detected, without waiting
for the rest of the head. The whole request may not exceed **67108864** bytes
(64 MiB): a `Content-Length` past that, or a body that grows past it, is answered
`413 Payload Too Large`.

**Parsing.** The request line yields an uppercased `method` and a request target.
The target is split at the first `?`: the part before it is percent-decoded into
`http::Request.path` (falling back to the raw text if decoding fails), the part after it
is parsed into `http::Request.query`; `http::Request.rawPath` keeps the target as received.
Header field names are lowercased and duplicates collapse last-wins, except that
the header block is parsed strictly so it can never be read two ways: a second
`Content-Length`, a `Content-Length` alongside `Transfer-Encoding`, whitespace
between a field name and its colon, an obsolete folded continuation line, a
`Content-Length` that is not an unsigned decimal number, or a `Transfer-Encoding`
whose final coding is not exactly `chunked` is answered `400 Bad Request`.
The body is exactly the framed bytes: truncated to `Content-Length`, or
de-chunked. A `multipart/form-data` body is split into `http::Request.parts`
keyed by each part's `name`. `http::Request.body` holds the raw body bytes.

**Matching.** Routes are tested in list order and the **first** match wins. Path
matching is segment-based on the decoded path with a single trailing `/` ignored;
`:name` binds one required segment, `:name?` binds an optional trailing segment,
and `*` binds all remaining segments joined by `/`. Bound captures are placed in
`http::Request.params` (the wildcard under the key `"*"`) before the handler runs.

**Crash-proofing.** The accept loop never dies on a bad client. A handler that
fails for any reason is answered with a built-in `500 Internal Server Error`, as
is a handler whose response carries a control byte (CR, LF, NUL, ...) in its
reason phrase or in a header name or value — such a response could otherwise be
split by a reflected value, so it is never serialized; a path matching no route
is answered with `404 Not Found`; an unparsable request line, header block, or
chunk-size line is answered with `400 Bad Request`; an over-cap request is
answered with `413 Payload Too Large` or `431 Request Header Fields Too Large`.
Every step after the accept is trapped, so a malformed request can only ever
cost its own connection, never the process. A write that fails mid-response
drops the connection and returns normally.

**Emission.** The status line is `HTTP/1.1 <status> <reason>`; an empty
`http::Response.reason` is filled in from a built-in table keyed by status code, falling
back to `OK` below 300, `Redirect` below 400, `Client Error` below 500, and
`Server Error` otherwise. Handler-set `Content-Length` and `Connection` headers
are dropped so framing stays correct, and the server always emits its own
`Content-Length` (the byte length of `http::Response.body`) plus `Connection: close`.
The body is written only when it is non-empty."#;

const EX: &str = r#"A plaintext accept loop with one route:

```
IMPORT http
IMPORT tcp
IMPORT collections

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("hello from " & req.path)
END FUNC

SUB main()
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/", home))
  RES s AS tcp::Listener = http::server(8080)
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

SUB main()
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/user/:id", showUser))
  RES s AS tls::Listener = http::serverSSL(8443, "cert.pem", "key.pem")
  DO
    http::handleRequest(s, routes)
  LOOP UNTIL FALSE
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Plaintext accept -> read -> parse -> dispatch -> respond -> close (one connection).
' Only a pointer to the listener is passed (it stays open for the next call); the accepted socket
' belongs to the call and is closed when it returns.
' bug-507 / OS-51: everything after the accept is trapped. The bounded read loop
' reports its outcome; an unexpected raise anywhere in reading, parsing, or
' serializing is answered 500 or drops this one connection — never the process.
SUB __http_handleRequest(RES listener AS tcp::Listener, routes AS List OF Route)
  RES sock AS tcp::Socket = tcp::accept(listener)
  MUT rr AS __http_ReadResult = __http_ReadResult[[], 0]
  rr = __http_readRequestNet(sock) TRAP(e)
    RECOVER __http_ReadResult[[], 500]
  END TRAP
  IF len(rr.raw) = 0 THEN
    EXIT SUB
  END IF
  MUT resp AS Response = __http_status(500, "Internal Server Error")
  IF rr.status = 0 THEN
    resp = __http_buildResponse(rr.raw, routes) TRAP(e)
      RECOVER __http_status(500, "Internal Server Error")
    END TRAP
  ELSE
    resp = __http_status(rr.status, __http_reasonPhrase(rr.status))
  END IF
  MUT head AS String = ""
  head = __http_serializeHead(resp) TRAP(e)
    EXIT SUB
  END TRAP
  tcp::write(sock, head) TRAP(e)
    EXIT SUB
  END TRAP
  IF len(resp.body) > 0 THEN
    tcp::write(sock, resp.body) TRAP(e)
      EXIT SUB
    END TRAP
  END IF
  IF rr.status <> 0 THEN
    __http_lingerNet(sock)
  END IF
END SUB"#;

#[rustfmt::skip]
const BODY_SSL: &str =
r#"' TLS counterpart: identical core, `tls::` transport (server-side handshake in
' tls::accept). The two bodies cannot share one socket variable (§F.5.6).
SUB __http_handleRequestSSL(RES listener AS tls::Listener, routes AS List OF Route)
  RES sock AS tls::Socket = tls::accept(listener)
  MUT rr AS __http_ReadResult = __http_ReadResult[[], 0]
  rr = __http_readRequestTls(sock) TRAP(e)
    RECOVER __http_ReadResult[[], 500]
  END TRAP
  IF len(rr.raw) = 0 THEN
    EXIT SUB
  END IF
  MUT resp AS Response = __http_status(500, "Internal Server Error")
  IF rr.status = 0 THEN
    resp = __http_buildResponse(rr.raw, routes) TRAP(e)
      RECOVER __http_status(500, "Internal Server Error")
    END TRAP
  ELSE
    resp = __http_status(rr.status, __http_reasonPhrase(rr.status))
  END IF
  MUT head AS String = ""
  head = __http_serializeHead(resp) TRAP(e)
    EXIT SUB
  END TRAP
  tls::write(sock, head) TRAP(e)
    EXIT SUB
  END TRAP
  IF len(resp.body) > 0 THEN
    tls::write(sock, resp.body) TRAP(e)
      EXIT SUB
    END TRAP
  END IF
  IF rr.status <> 0 THEN
    __http_lingerTls(sock)
  END IF
END SUB"#;

fn overload(listener_ty: &'static str, body: Body) -> Implementation {
    Implementation {
        params: vec![
            Parameter {
                name: "listener",
                desc: "An open listening socket to accept one connection from. It stays open and usable after the call. Must be bound with `RES`.",
                aliases: &["server"],
                ty: ParameterType::named(listener_ty),
                default: crate::codegen::registry::DefaultValue::None,
            },
            super::req("routes", "Routes tested in list order, first match wins. Build entries with `http::route`. An empty list makes every request a `404`.",
                &[],
                ParameterType::list_of(ParameterType::named(super::ROUTE_TYPE)),
            ),
        ],
        return_type: ParameterType::Nothing,
        errors: vec![],
        body,
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "handleRequest",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("tcp::Listener or tls::Listener, List OF Route"),
        internal_only: false,
        implementations: vec![
            overload(
                super::LISTENER_TYPE,
                Body::mfb(BODY, "__http_handleRequest"),
            ),
            overload(
                super::TLS_LISTENER_TYPE,
                Body::mfb(BODY_SSL, "__http_handleRequestSSL"),
            ),
        ],
    });
}
