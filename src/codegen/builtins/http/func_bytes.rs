//! `http::bytes` — descriptor entry (source-backed, body `__http_bytes`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str =
    r#"UTF-8 encode a `String` into the byte list a request or response body holds"#;

const DESC: &str = r#"`http::bytes` encodes `text` as UTF-8 and returns the result as a
`List OF Byte`, which is the type of the `body` field on both `http::Response`
and `http::Request`. It is a direct wrapper over `strings::toBytes`, so the
result is exactly the raw UTF-8 bytes backing the string — one list element per
byte, not per character.

The encoding is unconditional and lossless in both directions: nothing is
escaped, trimmed, length-limited, or inspected, and no header is set or implied.
An empty `String` yields an empty `List OF Byte`. `toString` on a
`List OF Byte` is the inverse, which is how a received `body` is read back as
text.

This exists for the case where you are editing a body directly — typically with
`WITH` on an existing response — because the field is bytes and a `String`
cannot be assigned to it. When you are *constructing* a response you do not need
it: `http::ok`, `http::status`, and `http::json` all take a `String` body and
encode it for you.

`http::bytes` is a pure function. It reads no state, performs no I/O, and
mutates nothing; the same input always produces the same output."#;

const EX: &str = r#"Replace the body of an existing response:

```
IMPORT http
IMPORT io

SUB main()
  LET base AS http::Response = http::status(418, "")
  LET resp AS http::Response = WITH base { body := http::bytes("I'm a teapot") }
  io::print(toString(resp.status) & " " & toString(len(resp.body)) & " bytes")
END SUB
```

prints:

```
418 12 bytes
```

Round-trip a body back to text:

```
IMPORT http
IMPORT io

SUB main
  LET body AS List OF Byte = http::bytes("hello")
  io::print(toString(len(body)))
  io::print(toString(body))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_bytes(text AS String) AS List OF Byte
  RETURN strings::toBytes(text)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req(
                "text",
                "The text to encode. Any string is accepted, including the empty string.",
                &[],
                ParameterType::String,
            )],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: Body::mfb(BODY, "__http_bytes"),
        }],
    });
}
