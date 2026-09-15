//! `encoding::base64Decode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Decode a standard Base64 `String` into a `List OF Byte`."#;
const DESC: &str = r#"`encoding::base64Decode` decodes `text` written in the standard Base64 alphabet
(RFC 4648 §4) and returns the bytes it encodes. Each non-padding character selects
a 6-bit value from the alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`; the values are
concatenated most-significant bit first into a continuous bit stream and emitted
eight bits at a time, so leftover bits that do not fill a final byte are discarded.
It decodes everything `encoding::base64Encode` writes, and it is more lenient than
the RFC: it does not check the padding count or that discarded bits are zero, so
`"===="` decodes to the empty list and `"AB=="` decodes to one zero byte, which
`base64Encode` writes back as `"AA=="`. Do not use a decode round-trip to check that
text is canonical Base64.

The alphabet is the standard variant using `+` and `/` for values `62` and `63`;
it is case-sensitive (`A`–`Z` map to `0`–`25`, `a`–`z` to `26`–`51`, `0`–`9` to
`52`–`61`). The `=` character is treated as padding: once a `=` is seen, any
later non-padding character is rejected. Padding characters are otherwise ignored
and contribute no bits.

The total input length (including padding) must be a multiple of four
characters. In addition, the number of non-padding symbols cannot be exactly one
more than a multiple of four (a symbol count whose remainder modulo four is `1`),
because no well-formed Base64 group ends on a single 6-bit symbol. Every rejection
— an invalid character, a symbol after `=`, or a bad length — raises
`ErrInvalidFormat`. The empty string decodes to the empty list. For the URL- and
filename-safe variant that
uses `-` and `_`, use `encoding::base64UrlDecode`."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_base64Decode(text AS String) AS List OF Byte
  LET data AS List OF Byte = strings::toBytes(text)
  LET total AS Integer = len(data)
  IF total - (total / 4) * 4 <> 0 THEN
    FAIL error(77050003, "invalid base64 length")
  END IF
  LET values AS List OF Integer = __encoding_base64Symbols(text, FALSE)
  LET symbols AS Integer = len(values)
  IF symbols - (symbols / 4) * 4 = 1 THEN
    FAIL error(77050003, "invalid base64 length")
  END IF
  RETURN __encoding_baseDecodeBits(values, 6)
END FUNC"#;
const EX: &str = r#"Decode a Base64 string back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::base64Decode("aGVsbG8=")
  io::print(encoding::utf8Decode(bytes))
END SUB
```

Round-trip through `base64Encode`:

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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "base64Decode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "text",
                desc: "The Base64 text to decode.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec!["ErrInvalidFormat"],
            body: Body::mfb(BODY, "__encoding_base64Decode"),
        }],
    });
}
