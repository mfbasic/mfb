//! `encoding::base64UrlEncode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/base64UrlEncode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Encode a `List OF Byte` to a URL- and filename-safe Base64 `String`."#;
const DESC: &str = r#"`encoding::base64UrlEncode` returns the URL- and filename-safe Base64
representation of `data` as defined by RFC 4648 §5. Input bytes are consumed as
a continuous bit stream, most-significant bit first, and emitted six bits at a
time; each 6-bit group selects one character from the alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_`, so the
result uses `-` and `_` for the final two symbols instead of the `+` and `/`
used by the standard variant.

Encoding operates on 24-bit (3-byte) groups, each producing four Base64
characters. When the final group is short, the remaining data bits occupy the
high-order bits of the last symbol and the low-order bits are zero-filled, but
**no** `=` padding characters are appended, so the output length is not rounded
up to a multiple of four. This is the difference from `encoding::base64Encode`,
which pads with `=`. An empty list yields the empty
string.

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. The inverse
operation is `encoding::base64UrlDecode`, which parses a URL-safe Base64 string
back into a `List OF Byte`."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_base64UrlEncode(data AS List OF Byte) AS String
  RETURN __encoding_baseEncode(data, "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_", 6, 4, FALSE)
END FUNC"#;
const EX: &str = r#"Encode bytes to URL-safe Base64:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::base64UrlEncode(raw))
END SUB
```

Round-trip through `base64UrlDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base64UrlEncode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base64UrlDecode(text)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "base64UrlEncode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "data",
                desc: "The bytes to encode.",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::Byte),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::mfb(BODY, "__encoding_base64UrlEncode"),
        }],
    });
}
