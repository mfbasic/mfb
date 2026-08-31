//! `http::finish` — descriptor entry (source-backed, body `__http_finish`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Parse a completed HTTP stream's accumulated bytes into a `Response`."#;

const DESC: &str = r#"`finish` turns the bytes accumulated in `stream.state.raw` into an
`http::Response`. Call it once `http::done` reports the exchange complete. If a
transport failure was captured during the drive (`state.err <> 0`), `finish`
`FAIL`s with that error; otherwise it parses the accumulated bytes with the same
parser the blocking `http::read` uses — status line, header block (field names
lowercased, duplicates last-wins), and body (de-chunked when the reply was
`chunked`).

`finish` does not close the stream: the handle stays bound and its socket is
closed exactly once when its binding goes out of scope. The returned `Response` is a plain,
copyable value record — `status`, `reason`, `httpVersion`, `headers`, `body`, and
`ok` (`TRUE` only for a 2xx status) — identical to what a blocking `http::read`
over the same URL would return. Redirects are not followed; a 3xx reply is
returned as-is with `ok` `FALSE`."#;

const EX: &str = r#"```
IMPORT net
IMPORT http
IMPORT io

SUB main()
  RES s AS http::Stream STATE PendingState = http::startRead(net::toUrl("http://example.com/"))
  WHILE http::done(s) = FALSE
    IF http::ready(s) THEN
      http::pump(s)
    END IF
  END WHILE
  LET r AS http::Response = http::finish(s)
  io::print(toString(r.status) & " " & r.reason)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Parse the accumulated bytes into a `Response`, or FAIL if a transport error was
' captured. The stream stays bound and is closed by the caller's drop.
FUNC __http_finish(RES s AS Stream STATE PendingState) AS Response
  IF s.state.err <> 0 THEN
    FAIL error(s.state.err, "http stream transport failed")
  END IF
  RETURN __http_parseResponse(s.state.raw)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "finish",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Stream STATE PendingState"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("stream", "The completed stream from `http::startRead` (after `http::done` is `TRUE`). The stream stays open — `finish` reads it and leaves it to you to close.", &[], ParameterType::named("Stream"))],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__http_finish"),
        }],
    });
}
