//! `encoding::base64Encode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/base64Encode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, RegistryFunction,
    RegistryPackage,
};

const INTRO: &str = r#"Encode a `List OF Byte` to a standard Base64 `String`."#;
const DESC: &str = r#"`encoding::base64Encode` returns the standard Base64 representation of `data`
as defined by RFC 4648 §4. Input bytes are consumed as a continuous bit stream,
most-significant bit first, and emitted six bits at a time; each 6-bit group
selects one character from the alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`, so the
result uses `+` and `/` for the final two symbols.

Encoding operates on 24-bit (3-byte) groups, each producing four Base64
characters. When the final group is short, the remaining data bits occupy the
high-order bits of the last symbol and the low-order bits are zero-filled, and
the output is then padded with `=` characters until its length is a multiple of
four, so the result length is always a multiple of four. An empty list yields
the empty string.

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. For the URL- and
filename-safe variant that uses `-` and `_` without `=` padding, use
`encoding::base64UrlEncode`. The inverse operation is `encoding::base64Decode`,
which parses a Base64 string back into a `List OF Byte`."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_base64Encode(data AS List OF Byte) AS String
  RETURN __encoding_baseEncode(data, "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/", 6, 4, TRUE)
END FUNC"#;
const EX: &str = r#"Encode bytes to Base64:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::base64Encode(raw))
END SUB
```

Round-trip through `base64Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base64Encode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base64Decode(text)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "base64Encode",
        intro: INTRO,
        desc: DESC,
        example: EX,
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
            lowering: Lowering::Helper,
            body: Body::mfb(BODY, "__encoding_base64Encode"),
        }],
    });
}
