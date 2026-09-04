//! `http::write` — descriptor entry (source-backed, body `__http_write`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Perform one blocking HTTP/1.1 request that carries a body and return the response."#;

const DESC: &str = r#"`write` performs exactly one blocking HTTP/1.1 request that carries a **body**
and returns the reply as a `http::Response` value. It opens a fresh connection
to `url.host` on `url.port` — plaintext through the `tcp` package for an `http://`
URL, TLS through the `tls` package for an `https://` URL — writes the request
line, headers, and body, reads the response to end of stream, closes the
connection, and returns. The connection is never reused; every call sends
`Connection: close`.

The `body` is sent verbatim as UTF-8 bytes. A `Content-Length` header equal to
the body's **byte** length is always generated, so a caller cannot override the
framing.

The `method` argument defaults to `POST` and may be any body-carrying verb
(`PUT`, `PATCH`, and so on). It is uppercased before it is sent, so `"put"` and
`"PUT"` are equivalent.

The optional `headers` map contributes request headers. A caller entry whose name
matches one of the automatic headers — `Host`, `User-Agent`, or `Accept` — replaces
that default (the match is case-insensitive); any other entry is appended verbatim.
The framing headers `Connection` and `Content-Length` are reserved: `Connection`
is always `close` and cannot be overridden, and `Content-Length` is always derived
from the body — a caller entry for either is dropped. Every header name and value,
along with the request target and `Host` derived from the URL, is rejected if it
contains a control byte (any byte below `0x20`, such as CR or LF), so a caller
cannot smuggle extra headers or a second request line.

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

const EX: &str = r#"POST a JSON body with an explicit content type, then read the status:

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
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_write(url AS net::Url, body AS String, headers AS Map OF String TO String, method AS String) AS Response
  RES s AS Stream STATE PendingState = __http_startExchange(url, body, TRUE, headers, method)
  WHILE __http_done(s) = FALSE
    __http_waitReadable(s)
    __http_pump(s)
  END WHILE
  RETURN __http_finish(s)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Url, String, Map OF String TO String, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("url", "The target URL. `url.scheme` selects transport (`https` → TLS on default port 443, otherwise plaintext on default port 80); `url.host`, `url.port`, `url.path`, and `url.query` form the connection and request target.", &[], ParameterType::named(crate::codegen::builtins::net::URL_TYPE_ID)),
                super::req("body", "The request payload, sent verbatim as UTF-8 bytes. Its byte length becomes the generated `Content-Length` header.", &[], ParameterType::String),
                super::fill("headers", "Optional request headers. Names matching `Host`/`User-Agent`/`Accept` override the defaults case-insensitively; others are appended. `Content-Length` and `Connection` entries are dropped (both are forced). No name or value may contain a control byte. Defaults to an empty map.", super::header_map(), "{}"),
                super::fill("method", "Optional request method; uppercased before sending. Must be non-empty and contain no space or control byte. Defaults to `POST`.", ParameterType::String, "POST"),
            ],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__http_write"),
        }],
    });
}
