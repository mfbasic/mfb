//! `encoding::codepageDecode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors the other `encoding` codecs). The descriptor carries an
//! MFBASIC source body. Body byte-significant (2-space indent → `.ncode` columns);
//! do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Decode a `List OF Byte` as text in a legacy single-byte codepage."#;
const DESC: &str = r#"`encoding::codepageDecode` reads `bytes` as text in `codepage` and returns the
`String` they spell. It is the decoder for content that is not UTF-8 — a
`windows-1252` page body, a `KOI8-R` mail part, an `IBM866` DOS file — where
`encoding::utf8Decode` rejects the same bytes as malformed.

Each codepage is one of the `encoding::Codepage` members; `mfb man encoding types`
lists them. For every single-byte codepage, bytes `0`–`127` are ASCII and decode to
themselves, and bytes `128`–`255` are looked up in that codepage's table.
`Codepage.Utf8` instead decodes the whole input as UTF-8, so a caller holding a
charset label has one entry point for both cases.

Some codepages leave part of their high half undefined: `windows-874` defines 120
of its 128 high bytes and `ISO-8859-6` only 83. A byte with no meaning in the
selected codepage is rejected with `ErrInvalidFormat` (`77050003`) rather than
replaced by `U+FFFD`, so a decode either spells the whole input or fails. If you
want a replacement character, catch the error and substitute it yourself.

The tables are the WHATWG Encoding Standard's legacy single-byte indexes, so the
text matches what a browser would show for the same bytes and the same label.
`encoding::codepageEncode` is the inverse."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_codepageDecode(codepage AS Codepage, bytes AS List OF Byte) AS String
  IF codepage = Codepage.Utf8 THEN
    RETURN __encoding_utf8Decode(bytes)
  END IF
  LET table AS String = __encoding_codepageTable(codepage)
  MUT out AS String = ""
  FOR EACH b IN bytes
    LET n AS Integer = toInt(b)
    IF n < 128 THEN
      out = out & __encoding_fromCodepoint(n)
    ELSE
      LET ch AS String = strings::mid(table, n - 128, 1)
      IF ch = "\u{FFFD}" THEN
        FAIL error(77050003, "byte not mapped in this codepage")
      END IF
      out = out & ch
    END IF
  NEXT
  RETURN out
END FUNC"#;
const EX: &str = r#"Read `windows-1252` bytes that are not valid UTF-8:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = [toByte(99), toByte(97), toByte(102), toByte(233)]
  io::print(encoding::codepageDecode(encoding::Codepage.Windows1252, bytes))
END SUB
```

Pick the codepage from a charset the caller already knows, with UTF-8 handled by
the same call:

```
IMPORT encoding
IMPORT io

SUB main()
  LET cyrillic AS List OF Byte = encoding::hexDecode("cff0e8e2e5f2")
  io::print(encoding::codepageDecode(encoding::Codepage.Windows1251, cyrillic))
  LET utf AS List OF Byte = encoding::utf8Encode("héllo")
  io::print(encoding::codepageDecode(encoding::Codepage.Utf8, utf))
END SUB
```

Fall back to a replacement character when a codepage leaves a byte undefined:

```
IMPORT encoding
IMPORT io

FUNC readOr(cp AS encoding::Codepage, bytes AS List OF Byte, fallback AS String) AS String
  RETURN encoding::codepageDecode(cp, bytes)
  TRAP(err)
    RETURN fallback
  END TRAP
END FUNC

SUB main()
  LET thai AS List OF Byte = [toByte(161), toByte(162)]
  LET hole AS List OF Byte = [toByte(219)]
  io::print(readOr(encoding::Codepage.Windows874, thai, "?"))
  io::print(readOr(encoding::Codepage.Windows874, hole, "?"))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "codepageDecode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "codepage",
                    desc: "The codepage to read the bytes in. `Codepage.Utf8` decodes the whole input as UTF-8 instead.",
                    aliases: &[],
                    ty: ParameterType::named("Codepage"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "bytes",
                    desc: "The bytes to decode. The empty list decodes to the empty string.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::Byte),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec!["ErrInvalidFormat"],
            body: Body::mfb(BODY, "__encoding_codepageDecode"),
        }],
    });
}
