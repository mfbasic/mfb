//! `http::status` — descriptor entry (source-backed, body `__http_status`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Build a plain-text response carrying an arbitrary status code."#;

const DESC: &str = r#"`status` is the general response constructor: it returns a new `http::Response`
with `status` set to `code`, an empty `reason`, `httpVersion` `"1.1"`, a
`headers` map holding the single entry `content-type` →
`"text/plain; charset=utf-8"`, a `body` of the UTF-8 bytes of `body`, and `ok`
set to `TRUE` exactly when `code` is in `200..299`.

It is the same constructor `http::ok` uses, with `code` left to you rather than
fixed at `200`. Like `ok`, it is a pure value constructor: it reads no state,
performs no I/O, and mutates nothing.

`code` is not validated. Any `Integer` is accepted and stored verbatim —
including `0`, a negative value, or a number outside the HTTP range — and it is
written into the status line unchanged when the response is served. Supplying a
non-HTTP code produces a malformed response rather than an error.

`reason` is left `""` on purpose. When the response is serialized, an empty
reason is replaced by a phrase derived from the status code: the common codes
(`200`, `201`, `202`, `204`, the `3xx` redirects, `400`, `401`, `403`, `404`,
`405`, `409`, `413`, `418`, `422`, `429`, `500`, `501`, `503`) map to their
standard phrases, and anything else falls back by class to `"OK"` below `300`,
`"Redirect"` below `400`, `"Client Error"` below `500`, and `"Server Error"`
otherwise. So `http::status(422, ...)` goes out as
`HTTP/1.1 422 Unprocessable Entity` without you setting a reason.

Two further details matter in practice:

- `ok` is a plain stored field, computed once here. A later
  `WITH resp { status := 500 }` leaves `ok` at whatever this call decided; set
  it yourself if anything downstream reads it.
- The header name is stored lowercased, as `content-type`. `http::withHeader`
  stores the name exactly as given, so overriding the content type means passing
  the lowercase `"content-type"`; `"Content-Type"` adds a *second* map entry and
  both are emitted.

`body` is not escaped, truncated, or inspected; an empty string produces a valid
response with a zero-length body. When the response is served, `Content-Length`
and `Connection` are always supplied by the server, and any handler-set value
for those two names is dropped.

The `http` package uses `status` internally for its own generated responses —
`404 Not Found` for an unmatched route, `400 Bad Request` and
`413 Payload Too Large` for framing failures, and `500 Internal Server Error`
when a handler traps.

For a `200` text response use `http::ok`, for JSON use `http::json`, and to add
further headers wrap the result with `http::withHeader`."#;

const EX: &str = r#"A validation-failure response from a handler:

```
IMPORT http

FUNC submit(req AS http::Request) AS http::Response
  RETURN http::status(422, "validation failed")
END FUNC
```

The `ok` field follows the status class:

```
IMPORT http
IMPORT io

SUB main()
  LET good = http::status(201, "created")
  LET bad = http::status(503, "try later")
  io::print(toString(good.ok) & " " & toString(bad.ok))
END SUB
```

A redirect, with the reason phrase supplied on emit and the location added
afterwards:

```
IMPORT http
IMPORT io

SUB main()
  LET resp = http::withHeader(http::status(302, ""), "location", "/login")
  io::print(toString(resp.status))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_status(code AS Integer, body AS String) AS Response
  RETURN __http_responseWith(code, body, "text/plain; charset=utf-8")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "status",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("code", "The HTTP status code stored in the response and written to the status line. Not range-checked; any `Integer` is accepted.", &[], ParameterType::Integer),
                super::req("body", "The response body. Encoded to bytes as UTF-8. Any string is accepted, including the empty string.", &[], ParameterType::String),
            ],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__http_status"),
        }],
    });
}
