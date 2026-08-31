//! `net::percentDecode` — descriptor entry + the `__net_percentDecode` MFBASIC source body
//! (`Body::mfb`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Percent-decode a URL path component."#;

const DESC: &str = r#"`net::percentDecode` decodes the `%XX` escapes in a request-target path component
and returns the result as a `String`. It walks `s` one grapheme at a time: a `%`
takes the next two characters and contributes the byte they name in
hexadecimal, and every other grapheme contributes its own UTF-8 bytes unchanged.
The accumulated bytes are then validated as UTF-8, so the result is always
well-formed text.

Unlike query decoding, a literal `+` is left as a `+`. A `+` in a path segment is
an ordinary character, not a space; only `application/x-www-form-urlencoded`
query data gives it that meaning. Use `net::parseQuery` for a query string, whose
keys and values do decode `+` to a space.

Decoding here is **strict**, which is the other way it differs from
`net::parseQuery`. A `%` with fewer than two characters after it, a `%` followed
by something that is not a hexadecimal pair, or a decoded byte sequence that is
not valid UTF-8 all raise `ErrInvalidFormat`. That is the only error this
function raises — nothing else escapes with a different code.

This is the decoder the built-in `http` server applies to a request path."#;

const EX: &str = r#"Decode an escaped path:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  io::print(net::percentDecode("/a%20b/c"))
  RETURN 0
END FUNC
```

Report the error code for a malformed escape:

```
IMPORT net
IMPORT io

FUNC decodeOrCode(s AS String) AS String
  RETURN net::percentDecode(s)
  TRAP(e)
    RETURN toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print(decodeOrCode("/a%20b"))
  io::print(decodeOrCode("%2"))
END SUB
```

prints the decoded text, then the error code for the truncated escape:

```
/a b
77050003
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Percent-decode a request-target path component: `%XX` -> bytes, UTF-8
' validated. Unlike query decoding, `+` is left literal (a path segment's `+`
' is not a space). A malformed escape or non-UTF-8 result fails
' `ErrInvalidFormat`.
FUNC __net_percentDecode(s AS String) AS String
  RETURN __net_percentDecodeImpl(s, FALSE)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "percentDecode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("s", "The percent-encoded path component to decode. Also accepted under the alternate named-argument spellings `text` and `value`, so `net::percentDecode(s := p)`, `net::percentDecode(text := p)`, and `net::percentDecode(value := p)` all bind position 0.", &["text", "value"], ParameterType::String)],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::mfb(BODY, "__net_percentDecode"),
        }],
    });
}
