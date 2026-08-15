//! `encoding::hexDecode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/hexDecode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, RegistryFunction,
    RegistryPackage,
};

const INTRO: &str = r#"Decode a hexadecimal `String` into a `List OF Byte`."#;
const DESC: &str = r#"`encoding::hexDecode` parses `text` as base-16 and returns the bytes it encodes.
Every two hexadecimal characters produce one byte: the first character is the
high nibble and the second is the low nibble, so the byte value is
`high * 16 + low`. Characters are consumed in order with no separators, prefix,
or padding recognized.

Both cases are accepted for the letter digits: `0`–`9`, `a`–`f`, and `A`–`F` are
valid, and lowercase and uppercase may be mixed freely within the same string.
Any other character is rejected.

The input length must be even, because each byte needs a pair of digits. The
empty string decodes to the empty list. The result always contains exactly half
as many bytes as there are input characters. This is the inverse of
`encoding::hexEncode`, which emits lowercase hex; decoding then re-encoding a
valid string reproduces its lowercase form."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_hexDecode(text AS String) AS List OF Byte
  LET data AS List OF Byte = strings::toBytes(text)
  LET n AS Integer = len(data)
  IF n - (n / 2) * 2 <> 0 THEN
    FAIL error(77050003, "odd-length hex")
  END IF
  MUT result AS List OF Byte = []
  MUT i AS Integer = 0
  MUT hi AS Integer = 0
  MUT lo AS Integer = 0
  WHILE i < n
    hi = __encoding_hexValue(toInt(collections::get(data, i)))
    lo = __encoding_hexValue(toInt(collections::get(data, i + 1)))
    IF hi < 0 OR lo < 0 THEN
      FAIL error(77050003, "invalid hex digit")
    END IF
    result = collections::append(result, toByte(hi * 16 + lo))
    i = i + 2
  END WHILE
  RETURN result
END FUNC"#;
const EX: &str = r#"Decode a hex string to bytes and back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::hexDecode("68656c6c6f")
  io::print(encoding::utf8Decode(bytes))
END SUB
```

Round-trip through `hexEncode`, mixing digit case on input:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  LET hex AS String = encoding::hexEncode(raw)
  io::print(hex)
  io::print(encoding::utf8Decode(encoding::hexDecode("6869")))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hexDecode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "text",
                desc: "The hexadecimal text to decode.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::mfb(BODY, "__encoding_hexDecode"),
        }],
    });
}
