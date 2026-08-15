//! `encoding::formUrlDecode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/formUrlDecode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, RegistryFunction,
    RegistryPackage,
};

const INTRO: &str = r#"Decode `application/x-www-form-urlencoded` text back into a `String`."#;
const DESC: &str = r#"`encoding::formUrlDecode` reverses `encoding::formUrlEncode`, parsing
`application/x-www-form-urlencoded` data — the format HTML forms apply to
query-string values — back into text. The input is read as its UTF-8 byte
sequence and scanned left to right, producing a sequence of decoded bytes.


Each byte is handled as follows:

- A `%` (byte 37) begins a three-character escape `%XX`, where `XX` is two
  hexadecimal digits. The two digits are decoded (case-insensitively) into a
  single byte and the scan advances past all three characters.
- A `+` (byte 43) is replaced by a single space (byte 32). This is the one
  behavior that distinguishes form decoding from `encoding::percentDecode`,
  which leaves `+` untouched.
- Every other byte is copied through unchanged.

After the whole input has been decoded, the resulting byte sequence is
validated as UTF-8 and returned as a `String`. The empty string decodes to the
empty string. Hexadecimal digits in escapes may be upper- or lowercase."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_formUrlDecode(text AS String) AS String
  RETURN __encoding_percentDecodeBytes(text, TRUE)
END FUNC"#;
const EX: &str = r#"Decode a form field value, turning `+` into a space:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::formUrlDecode("name+%3D+a+b"))
END SUB
```

Round-trip through `formUrlEncode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET enc AS String = encoding::formUrlEncode("café & tea")
  io::print(enc)
  io::print(encoding::formUrlDecode(enc))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "formUrlDecode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The form-url-encoded text to decode.",
                aliases: &["text"],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::mfb(BODY, "__encoding_formUrlDecode"),
        }],
    });
}
