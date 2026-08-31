//! `http::done` — descriptor entry (source-backed, body `__http_done`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Report whether an HTTP stream's response is complete."#;

const DESC: &str = r#"`done` returns `TRUE` when the exchange has finished and no further `http::pump`
is needed — the drive loop's exit condition. It is `TRUE` when any of three things
holds: a transport error was captured (`state.err <> 0`); the peer closed the
connection (`state.closed`, the `Connection: close` terminator); or the bytes
accumulated so far already form a complete response frame (Content-Length
satisfied, or the final `chunked` chunk seen). The frame check is an early-out, so
a well-framed reply completes before the peer's EOF is observed.

`done` is a pure predicate over `stream.state`: it neither reads the socket nor
mutates STATE. Call it at the top of the drive loop; once it is `TRUE`, call
`http::finish` to obtain the `Response`."#;

const EX: &str = r#"```
IMPORT net
IMPORT http

SUB main()
  RES s AS http::Stream STATE PendingState = http::startRead(net::toUrl("http://example.com/"))
  WHILE http::done(s) = FALSE
    IF http::ready(s) THEN
      http::pump(s)
    END IF
  END WHILE
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' `TRUE` iff the response is complete: a failure, clean EOF, or a fully-framed
' response (Content-Length satisfied / final chunk — an early-out before EOF).
FUNC __http_done(RES s AS Stream STATE PendingState) AS Boolean
  IF s.state.err <> 0 THEN
    RETURN TRUE
  END IF
  IF s.state.closed THEN
    RETURN TRUE
  END IF
  RETURN __http_frameComplete(s.state.raw)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "done",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Stream STATE PendingState"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("stream", "The bound stream from `http::startRead`. The stream stays open — `done` only reads its state and leaves it to you to close.", &[], ParameterType::named("Stream"))],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::mfb(BODY, "__http_done"),
        }],
    });
}
