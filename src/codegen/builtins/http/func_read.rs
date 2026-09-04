//! `http::read` — descriptor entry + the `__http_read` MFBASIC source body
//! (`Body::mfb`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Perform one blocking, body-less HTTP/1.1 request and return the response."#;

const DESC: &str = r#"`read` performs exactly one blocking HTTP/1.1 request that carries **no body**
and returns the reply as a `http::Response` value. It opens a fresh connection
to `url.host` on `url.port` — plaintext through the `tcp` package for an `http://`
URL, TLS through the `tls` package for an `https://` URL — writes the request,
reads the response to end of stream, closes the connection, and returns. The
connection is never reused; every call sends `Connection: close`.

The `method` argument defaults to `GET` and may be any body-less verb (`HEAD`,
`DELETE`, `OPTIONS`, and so on). It is uppercased before it is sent, so `"get"`
and `"GET"` are equivalent. It must be non-empty and, like a header, may not
contain a space or a control byte (any byte below `0x20`, such as CR or LF) —
the method is the first token on the wire, so one with `\r\n` in it would
inject extra headers or a second request line; such a method raises
`ErrInvalidArgument`.

The optional `headers` map contributes request headers. A caller entry whose name
matches one of the automatic headers — `Host`, `User-Agent`, or `Accept` — replaces
that default (the match is case-insensitive); any other entry is appended verbatim.
The framing headers `Connection` and `Content-Length` are reserved: `Connection`
is always `close` and cannot be overridden, and no body means no `Content-Length`
is sent. Every header name and value, along with the request target and `Host`
derived from the URL, is rejected if it contains a control byte (any byte below
`0x20`, such as CR or LF), so a caller cannot smuggle extra headers or a second
request line.

The request target is `url.path` (an empty path is normalized to `/`) followed by
`?` and `url.query` when a query is present; the URL fragment is never sent.

The returned `http::Response` exposes `status` (Integer), `reason` (String, `""` when
omitted), `httpVersion` (String, e.g. `"1.1"`), `headers` (a `Map OF String TO
String`), `body` (a `List OF Byte`), and `ok` (Boolean, `TRUE` only when `status`
is in `200..299`). Header field names in `headers` are lowercased and duplicates
collapse last-wins, so read a header with the ordinary collections accessors, e.g.
`collections::getOr(resp.headers, "content-type", "")`. Redirects are **not**
followed: a 3xx reply is returned as-is, with `ok` `FALSE` and its target in
`resp.headers` under `"location"`. A `chunked` transfer-encoded body is de-chunked
before it is placed in `body`.

The client applies a 30-second connect deadline and, for plaintext, a 30-second
per-read deadline so a stalled or black-holed peer fails cleanly rather than
wedging the calling thread; the 64 MiB response cap bounds memory for a peer that
streams without end."#;

const EX: &str = r#"A plain GET, reading the status line:

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
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' plan-76-D: the blocking client is now a thin driver over the non-blocking core.
' It produces the SAME `http::Response` as the pre-plan-76-D `__http_exchange` path —
' identical request (`__http_buildRequest`), identical accumulated bytes, identical
' parse (`__http_parseResponse`) — only the read loop's shape differs (readiness-
' gated `pump` vs a direct `tcp::read` loop). `__http_waitReadable` preserves the
' read deadline; the socket closes itself exactly once when `s` goes out of scope.
FUNC __http_read(url AS net::Url, headers AS Map OF String TO String, method AS String) AS Response
  RES s AS Stream STATE PendingState = __http_startExchange(url, "", FALSE, headers, method)
  WHILE __http_done(s) = FALSE
    __http_waitReadable(s)
    __http_pump(s)
  END WHILE
  RETURN __http_finish(s)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "read",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Url, Map OF String TO String, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("url", "The target URL. `url.scheme` selects transport (`https` → TLS on default port 443, otherwise plaintext on default port 80); `url.host`, `url.port`, `url.path`, and `url.query` form the connection and request target.", &[], ParameterType::named(crate::codegen::builtins::net::URL_TYPE_ID)),
                super::fill("headers", "Optional request headers. Names matching `Host`/`User-Agent`/`Accept` override the defaults case-insensitively; others are appended. No name or value may contain a control byte. Defaults to an empty map.", super::header_map(), "{}"),
                super::fill("method", "Optional request method; uppercased before sending. Must be non-empty and contain no space or control byte. Defaults to `GET`.", ParameterType::String, "GET"),
            ],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__http_read"),
        }],
    });
}
