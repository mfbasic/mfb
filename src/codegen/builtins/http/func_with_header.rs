//! `http::withHeader` — descriptor entry (source-backed, body `__http_withHeader`).
//!

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Return a copy of a response with one header name set to a value"#;

const DESC: &str = r#"`http::withHeader` returns a new `http::Response` that is a copy of `resp` with
`name` mapped to `value` in its `headers` map. Every other field — `status`,
`reason`, `httpVersion`, `body`, `ok` — is carried over unchanged. `resp` itself
is not modified; `http::Response` is a plain copyable value record, so this is sugar
over `WITH resp { headers := ... }` and calls chain naturally.

The header map is an ordinary `Map OF String TO String`, and the name is used as
the map key **exactly as given**, with no case normalization. Two consequences
follow, and both bite in practice:

- Setting a name that is already present replaces its value. Setting a name that
  differs only in case *adds a second entry*, and both are emitted on the wire.
  The response constructors store their content type lowercased as
  `content-type`, so overriding it means passing `"content-type"` — passing
  `"Content-Type"` sends two content-type headers.
- Response header names go out on the wire spelled the way you wrote them. This
  is the opposite of the request side, where field names are lowercased during
  parsing, so a handler reads request headers in lowercase but writes response
  headers in whatever case it chooses.

Two names cannot be set this way. `Content-Length` and `Connection` are framing
headers the server always supplies itself; when the response is serialized, any
entry whose name matches either of them case-insensitively is dropped, and the
server's own correct values are appended. Setting them here is therefore silently
ineffective rather than an error.

`name` and `value` are stored verbatim — not validated, escaped, or scanned for
control characters. Do not build a header value out of unvalidated request data
without checking it yourself.

The first parameter is also accepted under the name `response`."#;

const EX: &str = r#"Add a caching directive to a text response:

```
IMPORT http
IMPORT io

SUB main()
  LET resp AS http::Response = http::withHeader(http::ok("pong"), "cache-control", "no-store")
  io::print(toString(resp.status) & " " & toString(len(resp.body)) & " bytes")
END SUB
```

prints:

```
200 4 bytes
```

Override the content type set by a constructor — note the lowercase name:

```
IMPORT http
IMPORT io

SUB main()
  LET base AS http::Response = http::ok("<h1>hi</h1>")
  LET resp AS http::Response = http::withHeader(base, "content-type", "text/html; charset=utf-8")
  io::print(toString(resp.status) & " " & toString(len(resp.body)) & " bytes")
END SUB
```

prints:

```
200 11 bytes
```

Chain several headers:

```
IMPORT http
IMPORT io

SUB main()
  MUT resp AS http::Response = http::json("{\"ok\":true}")
  resp = http::withHeader(resp, "x-request-id", "abc123")
  resp = http::withHeader(resp, "cache-control", "no-store")
  io::print(toString(resp.status) & " " & toString(len(resp.headers)) & " headers")
END SUB
```

prints:

```
200 3 headers
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_withHeader(resp AS Response, name AS String, value AS String) AS Response
  MUT h AS Map OF String TO String = resp.headers
  h = collections::set(h, name, value)
  RETURN WITH resp { headers := h }
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "withHeader",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Response, String, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("resp", "The response to copy. Not modified. Also accepted under the name `response`.",
                    &["response"],
                    ParameterType::named(super::RESPONSE_TYPE),
                ),
                super::req("name", "The header name, used as the map key exactly as written. Matching an existing entry replaces it; differing in case adds a second entry.", &[], ParameterType::String),
                super::req("value", "The header value, stored verbatim. Any string is accepted, including the empty string.", &[], ParameterType::String),
            ],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__http_withHeader"),
        }],
    });
}
