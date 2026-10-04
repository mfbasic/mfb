//! `http::finish` — descriptor entry (source-backed, body `__http_finish`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Parse a completed HTTP stream's accumulated bytes into a `http::Response`."#;

const DESC: &str = r#"`finish` turns the bytes accumulated in `stream.state.raw` into an
`http::Response`. Call it once `http::done` reports the exchange complete. If a
transport failure was captured during the drive (`state.err <> 0`), `finish`
`FAIL`s with that error; otherwise it parses the accumulated bytes with the same
parser the blocking `http::read` uses — status line, header block (field names
lowercased, duplicates last-wins), and body (de-chunked when the reply was
`chunked`).

`finish` does not close the stream: the handle stays bound and its socket is
closed exactly once when its binding goes out of scope. The returned `http::Response` is a plain,
copyable value record — `status`, `reason`, `httpVersion`, `headers`, `body`, and
`ok` (`TRUE` only for a 2xx status) — identical to what a blocking `http::read`
over the same URL would return. Redirects are not followed; a 3xx reply is
returned as-is with `ok` `FALSE`.

If a transport failure was recorded into `stream`'s STATE by an earlier
`http::pump`/the connect wait, `finish` raises that same error: `ErrInvalidArgument`
for a bad argument, `ErrNetworkFailed` for a socket-level failure, `ErrOverflow`
for exceeding the 64 MiB response cap, `ErrResourceClosed`/`ErrResourceMoved` for
a moved or already-closed resource, `ErrTimeout` for an expired deadline, or
`ErrTlsFailed` for an HTTPS record failure. A `state.err` holding any other
non-zero value — one the program wrote itself — is not a recorded failure, and
`finish` raises `ErrInvalidArgument` for it. Parsing a malformed response raises
`ErrInvalidFormat`, and reading its status line or headers can also raise
`ErrIndexOutOfRange`/`ErrNotFound`. `ErrOutOfMemory` means the program ran out
of memory."#;

const EX: &str = r#"```
IMPORT net
IMPORT http
IMPORT io

FUNC main() AS Nothing
  RES s AS http::Stream STATE http::PendingState = http::startRead(net::toUrl("http://example.com/"))
  WHILE http::done(s) = FALSE
    IF http::ready(s) THEN
      http::pump(s)
    END IF
  END WHILE
  LET r AS http::Response = http::finish(s)
  io::print(toString(r.status) & " " & r.reason)
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Parse the accumulated bytes into a `Response`, or FAIL if a transport error was
' captured. The stream stays bound and is closed by the caller's drop.
'
' `state.err` is a plain field the program can write (bug-808), so a captured code
' is re-raised only when it is one a drive records: `tcp::read`/`tls::read`'s
' failures except `ErrConnectionClosed` (which `__http_readNet`/`__http_readTls`
' turn into `closed`), `__http_waitReadable`'s `ErrTimeout`, and `__http_pump`'s
' `ErrOverflow` past the response cap. Any other non-zero value is not a transport
' failure and raises `ErrInvalidArgument`. Each code is a literal so the
' declared-errors guard sees exactly what `finish` can raise.
FUNC __http_finish(RES s AS Stream STATE PendingState) AS Response
  LET code AS Integer = s.state.err
  IF code = 0 THEN
    RETURN __http_parseResponse(s.state.raw)
  END IF
  IF code = 77050002 THEN
    FAIL error(77050002, "http stream transport failed")
  END IF
  IF code = 77070003 THEN
    FAIL error(77070003, "http stream transport failed")
  END IF
  IF code = 77010001 THEN
    FAIL error(77010001, "http stream transport failed")
  END IF
  IF code = 77050010 THEN
    FAIL error(77050010, "http stream transport failed")
  END IF
  IF code = 77030004 THEN
    FAIL error(77030004, "http stream transport failed")
  END IF
  IF code = 77030009 THEN
    FAIL error(77030009, "http stream transport failed")
  END IF
  IF code = 77050008 THEN
    FAIL error(77050008, "http stream transport failed")
  END IF
  IF code = 77070008 THEN
    FAIL error(77070008, "http stream transport failed")
  END IF
  FAIL error(77050002, "http stream state.err holds " & toString(code) & ", which is not a transport failure code")
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
            errors: vec!["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrInvalidFormat", "ErrNetworkFailed", "ErrNotFound", "ErrOutOfMemory", "ErrOverflow", "ErrResourceClosed", "ErrResourceMoved", "ErrTimeout", "ErrTlsFailed"],
            body: Body::mfb(BODY, "__http_finish"),
        }],
    });
}
