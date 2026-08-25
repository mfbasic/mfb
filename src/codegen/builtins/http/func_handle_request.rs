//! `http::handleRequest` — descriptor entry (source-backed). Overloaded by listener
//! type: a `net::Listener` rewrites to `__http_handleRequest`, a `tls::TlsListener`
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
user-owned `DO`/`LOOP`. The listener itself is **borrowed** — it stays open across
calls and is closed only by its own lexical drop (or `net::close` / `tls::close`).
The accepted socket is owned by the call and closed by lexical drop on return.

The server is single-threaded and blocking: the accept call blocks until a client
arrives, and one request is served at a time in the caller's loop. No timeout is
passed to the underlying `net::accept` / `tls::accept`, so the wait is unbounded.

**Reading.** Bytes are read in 64 KiB chunks and appended to a raw byte buffer
until the frame is complete — the header terminator `\r\n\r\n` has been seen and
the body implied by `Content-Length` has arrived, or, for
`Transfer-Encoding: chunked`, the `0\r\n\r\n` terminator has arrived. A read that
fails is treated as end of stream. If the peer closes before sending anything
(zero bytes read), the call returns without writing a response.

**Size cap.** The accumulated request may not exceed **67108864** bytes (64 MiB).
Once the buffer passes that size, reading stops and the connection is answered
with a `413 Payload Too Large`.

**Parsing.** The request line yields an uppercased `method` and a request target.
The target is split at the first `?`: the part before it is percent-decoded into
`Request.path` (falling back to the raw text if decoding fails), the part after it
is parsed into `Request.query`; `Request.rawPath` keeps the target as received.
Header field names are lowercased and duplicates collapse last-wins. A chunked
body is de-chunked, and a `multipart/form-data` body is split into
`Request.parts` keyed by each part's `name`. `Request.body` holds the raw body
bytes.

**Matching.** Routes are tested in list order and the **first** match wins. Path
matching is segment-based on the decoded path with a single trailing `/` ignored;
`:name` binds one required segment, `:name?` binds an optional trailing segment,
and `*` binds all remaining segments joined by `/`. Bound captures are placed in
`Request.params` (the wildcard under the key `"*"`) before the handler runs.

**Crash-proofing.** The accept loop never dies on a bad client. A handler that
fails for any reason is answered with a built-in `500 Internal Server Error`; a
path matching no route is answered with `404 Not Found`; an unparsable request
line or header block is answered with `400 Bad Request`; an over-cap request is
answered with `413 Payload Too Large`. A write that fails mid-response drops the
connection and returns normally.

**Emission.** The status line is `HTTP/1.1 <status> <reason>`; an empty
`Response.reason` is filled in from a built-in table keyed by status code, falling
back to `OK` below 300, `Redirect` below 400, `Client Error` below 500, and
`Server Error` otherwise. Handler-set `Content-Length` and `Connection` headers
are dropped so framing stays correct, and the server always emits its own
`Content-Length` (the byte length of `Response.body`) plus `Connection: close`.
The body is written only when it is non-empty."#;

const EX: &str = r#"A plaintext accept loop with one route:

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
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Plaintext accept -> parse -> dispatch -> respond -> close (one connection).
' Only a pointer to the listener is passed (it stays open for the next call); the accepted socket
' is owned and closed by lexical drop at return.
SUB __http_handleRequest(RES listener AS net::Listener, routes AS List OF Route)
  RES sock AS net::Socket = net::accept(listener)
  MUT raw AS List OF Byte = []
  MUT reading AS Boolean = TRUE
  MUT oversize AS Boolean = FALSE
  WHILE reading = TRUE
    MUT chunk AS List OF Byte = []
    chunk = net::read(sock, 65536) TRAP(e)
      RECOVER []
    END TRAP
    IF len(chunk) = 0 THEN
      reading = FALSE
    ELSE
      raw = collections::append(raw, chunk)
      IF len(raw) > __HTTP_MAX_REQUEST THEN
        oversize = TRUE
        reading = FALSE
      ELSEIF __http_frameComplete(raw) THEN
        reading = FALSE
      END IF
    END IF
  END WHILE
  IF len(raw) = 0 THEN
    EXIT SUB
  END IF
  MUT resp AS Response = __http_status(413, "Payload Too Large")
  IF oversize = FALSE THEN
    resp = __http_buildResponse(raw, routes)
  END IF
  net::writeText(sock, __http_serializeHead(resp)) TRAP(e)
    EXIT SUB
  END TRAP
  IF len(resp.body) > 0 THEN
    net::write(sock, resp.body) TRAP(e)
      EXIT SUB
    END TRAP
  END IF
END SUB"#;

#[rustfmt::skip]
const BODY_SSL: &str =
r#"' TLS counterpart: identical core, `tls::` transport (server-side handshake in
' tls::accept). The two bodies cannot share one socket variable (§F.5.6).
SUB __http_handleRequestSSL(RES listener AS tls::TlsListener, routes AS List OF Route)
  RES sock AS tls::TlsSocket = tls::accept(listener)
  MUT raw AS List OF Byte = []
  MUT reading AS Boolean = TRUE
  MUT oversize AS Boolean = FALSE
  WHILE reading = TRUE
    MUT chunk AS List OF Byte = []
    chunk = tls::read(sock, 65536) TRAP(e)
      RECOVER []
    END TRAP
    IF len(chunk) = 0 THEN
      reading = FALSE
    ELSE
      raw = collections::append(raw, chunk)
      IF len(raw) > __HTTP_MAX_REQUEST THEN
        oversize = TRUE
        reading = FALSE
      ELSEIF __http_frameComplete(raw) THEN
        reading = FALSE
      END IF
    END IF
  END WHILE
  IF len(raw) = 0 THEN
    EXIT SUB
  END IF
  MUT resp AS Response = __http_status(413, "Payload Too Large")
  IF oversize = FALSE THEN
    resp = __http_buildResponse(raw, routes)
  END IF
  tls::writeText(sock, __http_serializeHead(resp)) TRAP(e)
    EXIT SUB
  END TRAP
  IF len(resp.body) > 0 THEN
    tls::write(sock, resp.body) TRAP(e)
      EXIT SUB
    END TRAP
  END IF
END SUB"#;

fn overload(listener_ty: &'static str, body: Body) -> Implementation {
    Implementation {
        params: vec![
            Parameter {
                name: "listener",
                desc: "An open listening socket to accept one connection from. Borrowed — it remains open and usable after the call. Must be bound with `RES`.",
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
        expected_arguments: Some("Listener or TlsListener, List OF Route"),
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
