//! `http::ready` — descriptor entry (source-backed, body `__http_ready`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Report whether an HTTP stream has data available to read without blocking."#;

const DESC: &str = r#"`ready` returns `TRUE` when a non-blocking read of `stream` would return bytes or
observe end-of-stream right now, and `FALSE` when it would have to wait. It is a
pure readiness probe with a zero timeout — it never blocks and never reads
bytes — layered on the scalar `tcp::poll`/`tls::poll` of the active transport
variant. Use it to gate `http::pump` so a cooperative drive loop only reads when
progress is possible and otherwise does the caller's own work.

`ready` does not itself advance the exchange or change `stream`'s STATE; it only
reports readiness. A closed peer reads as ready (the terminating zero-byte read is
available), so a loop gated on `ready` still reaches `http::done`."#;

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
r#"' `TRUE` iff a non-blocking read would return bytes or EOF now.
FUNC __http_ready(RES s AS Stream STATE PendingState) AS Boolean
  MATCH s
    CASE tcp::Socket(p)
      RETURN tcp::poll(p, 0)
    CASE tls::Socket(t)
      RETURN tls::poll(t, 0)
  END MATCH
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "ready",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Stream STATE PendingState"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("stream", "The bound stream from `http::startRead`. The stream stays open — `ready` only reads its state and leaves it to you to close.", &[], ParameterType::named("Stream"))],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::mfb(BODY, "__http_ready"),
        }],
    });
}
