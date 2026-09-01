//! `http::startRead` — descriptor entry (source-backed, body `__http_startRead`).
//!

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Begin a non-blocking HTTP/1.1 GET-style exchange and return a drivable stream."#;

const DESC: &str = r#"`startRead` opens a connection, writes a body-less request, and returns
immediately with a bound `http::Stream` — a resource union over the plaintext
(`tcp::Socket`) and TLS (`tls::Socket`) transports — carrying a fresh
`http::PendingState`. It does **not** wait for the reply. The caller then drives the
exchange without blocking its thread: test `http::ready`, call `http::pump` to
read whatever bytes are available, repeat until `http::done`, and parse with
`http::finish`.

The transport is chosen from `url.scheme` exactly as `http::read` does: `https`
connects over the `tls` package (default port 443), anything else over plaintext
`tcp` (default port 80). The request is built by the same machinery as the
blocking client — `Connection: close` is always sent, `method` (default `GET`) is
uppercased, and the same control-byte rejection applies to every header name,
value, and the URL-derived request target and `Host`. The whole request is
written before `startRead` returns; `state.sentAll` is `TRUE`.

The returned handle is a `RES http::Stream STATE http::PendingState`: a resource
whose STATE accumulates the response across pumps. It stays bound and open — the
socket is closed exactly once when the handle goes out of scope — so a program reads
`state` through the handle while driving it. `http::read`/`http::write` are thin
blocking wrappers over this same core.

`startRead` applies the 30-second connect deadline; the per-read deadline is a
matter for the drive loop (`http::pump` never blocks; the blocking wrappers'
internal readiness wait bounds a stalled peer)."#;

const EX: &str = r#"Drive a GET cooperatively, interleaving other work:

```
IMPORT net
IMPORT http
IMPORT io

SUB main()
  RES s AS http::Stream STATE http::PendingState = http::startRead(net::toUrl("http://example.com/"))
  WHILE http::done(s) = FALSE
    IF http::ready(s) THEN
      http::pump(s)
    END IF
    ' ... the caller's own work happens here, uninterrupted ...
  END WHILE
  LET r AS http::Response = http::finish(s)
  io::print(toString(r.status) & " " & r.reason)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_startRead(url AS net::Url, headers AS Map OF String TO String, method AS String) AS RES Stream STATE PendingState
  RETURN __http_startExchange(url, "", FALSE, headers, method)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "startRead",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Url, Map OF String TO String, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("url", "The target URL. `url.scheme` selects transport (`https` → TLS on default port 443, otherwise plaintext on default port 80); `url.host`, `url.port`, `url.path`, and `url.query` form the connection and request target.", &[], ParameterType::named(crate::codegen::builtins::net::URL_TYPE_ID)),
                super::fill("headers", "Optional request headers. Names matching `Host`/`User-Agent`/`Accept` override the defaults case-insensitively; others are appended. No name or value may contain a control byte. Defaults to an empty map.", super::header_map(), "{}"),
                super::fill("method", "Optional request method; uppercased before sending. Must be non-empty and contain no space. Defaults to `GET`.", ParameterType::String, "GET"),
            ],
            return_type: super::stream_state(),
            errors: vec![],
            body: Body::mfb(BODY, "__http_startRead"),
        }],
    });
}
