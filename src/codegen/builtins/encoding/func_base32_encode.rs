//! `encoding::base32Encode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Encode a `List OF Byte` to a standard Base32 `String`."#;
const DESC: &str = r#"`encoding::base32Encode` returns the standard Base32 representation of `data`
as defined by RFC 4648 §6. Input bytes are read as a continuous bit stream,
most-significant bit first, and emitted five bits at a time; each 5-bit group
selects one character from the uppercase alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZ234567`.

Encoding operates on 40-bit (5-byte) groups, each producing eight Base32
characters. When the final group is short, its remaining bits become the high
bits of a last symbol and are zero-filled at the low end, then the output is
padded with `=` characters until its length is a multiple of eight, so the
result length is always a multiple of eight. An empty list yields the empty
string.

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. The inverse operation
is `encoding::base32Decode`, which parses a Base32 string back into a
`List OF Byte`."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_base32Encode(data AS List OF Byte) AS String
  RETURN __encoding_baseEncode(data, "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567", 5, 8, TRUE)
END FUNC"#;
const EX: &str = r#"Encode bytes to Base32:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::base32Encode(raw))
END SUB
```

Round-trip through `base32Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base32Encode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base32Decode(text)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "base32Encode",
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
            body: Body::mfb(BODY, "__encoding_base32Encode"),
        }],
    });
}
