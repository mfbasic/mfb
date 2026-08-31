//! `http::pump` — descriptor entry (source-backed, body `__http_pump`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Perform one non-blocking read on an HTTP stream, accumulating the response."#;

const DESC: &str = r#"`pump` does one non-blocking read of whatever bytes are available on `stream` and
appends them to `stream.state.raw`. It is internally gated on readiness (it calls
the same probe as `http::ready`), so it never blocks: when no bytes are available
it returns immediately having done nothing. A read that returns zero bytes marks
the stream `state.closed = TRUE` (the peer closed, the `Connection: close`
terminator); a transport failure is captured in `state.err` rather than raised,
so the drive loop stays simple and the error surfaces from `http::finish`.

Each call reads at most one 64 KiB chunk, so a large reply is accumulated across
several `pump` calls — the point of the cooperative API. If the accumulated
`state.raw` exceeds the internal 64 MiB response cap, `state.err` is set to the
overflow code and the exchange ends. `pump` is a `SUB`: it advances the stream's
STATE in place and returns nothing."#;

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
  io::print(toString(len(s.state.raw)) & " bytes accumulated")
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' One non-blocking read of available bytes (gated on readiness so it never
' blocks), appended to `s.state.raw`. The MATCH only CALLS a top-level read helper
' and assigns into the pre-declared `r` — no `<call> TRAP` binding inside the CASE
' (Correction D3). A 0-byte read sets `closed`; a transport failure sets `err`.
SUB __http_pump(RES s AS Stream STATE PendingState)
  IF __http_ready(s) = FALSE THEN
    EXIT SUB
  END IF
  MUT r AS __http_PumpRead = __http_PumpRead[[], FALSE, 0]
  MATCH s
    CASE tcp::Socket(p)
      r = __http_readNet(p, 65536)
    CASE tls::Socket(t)
      r = __http_readTls(t, 65536)
  END MATCH
  IF r.closed THEN
    s.state.closed = TRUE
  END IF
  IF r.err <> 0 THEN
    s.state.err = r.err
  END IF
  IF len(r.bytes) > 0 THEN
    s.state.raw = collections::append(s.state.raw, r.bytes)
  END IF
  IF len(s.state.raw) > __HTTP_MAX_RESPONSE THEN
    s.state.err = 77050010
  END IF
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pump",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Stream STATE PendingState"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("stream", "The bound stream from `http::startRead`. `pump` updates the stream in place and leaves it open — you still close it.", &[], ParameterType::named("Stream"))],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::mfb(BODY, "__http_pump"),
        }],
    });
}
