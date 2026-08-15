//! `encoding::hexEncode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/hexEncode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, RegistryFunction,
    RegistryPackage,
};

const INTRO: &str = r#"Encode a `List OF Byte` to a lowercase hexadecimal `String`."#;
const DESC: &str = r#"`encoding::hexEncode` returns the base-16 representation of `data`, emitting two
lowercase hexadecimal characters for every input byte with no separators, prefix,
or padding. Bytes are encoded in order: byte value `v` becomes the digit for
`v / 16` followed by the digit for the low nibble, drawn from the alphabet
`0123456789abcdef`.

The result length is always exactly twice the number of input bytes. An empty
list yields the empty string. Use `strings::upper` on the result if uppercase hex
is required.

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. The inverse operation
is `encoding::hexDecode`, which parses a hex string (accepting upper- or
lowercase digits) back into a `List OF Byte`."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_hexEncode(data AS List OF Byte) AS String
  MUT out AS String = ""
  MUT v AS Integer = 0
  FOR EACH b IN data
    v = toInt(b)
    out = out & __encoding_hexDigit(v / 16) & __encoding_hexDigit(v - (v / 16) * 16)
  NEXT
  RETURN out
END FUNC"#;
const EX: &str = r#"Encode bytes to lowercase hex:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::hexEncode(raw))
END SUB
```

Round-trip through `hexDecode`, and uppercase the digits:

```
IMPORT encoding
IMPORT strings
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  LET hex AS String = encoding::hexEncode(raw)
  io::print(strings::upper(hex))
  io::print(encoding::utf8Decode(encoding::hexDecode(hex)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hexEncode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
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
            body: Body::mfb(BODY, "__encoding_hexEncode"),
        }],
    });
}
