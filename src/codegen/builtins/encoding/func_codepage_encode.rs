//! `encoding::codepageEncode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors the other `encoding` codecs). The descriptor carries an
//! MFBASIC source body. Body byte-significant (2-space indent → `.ncode` columns);
//! do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Encode a `String` as bytes in a legacy single-byte codepage."#;
const DESC: &str = r#"`encoding::codepageEncode` writes `text` as bytes in `codepage`, and is the exact
inverse of `encoding::codepageDecode`: for any single-byte codepage, bytes that
decode without failing re-encode to exactly those bytes.

Each codepage is one of the `encoding::Codepage` members. For every single-byte
codepage, a character below `U+0080` is its own byte, and anything above is looked
up in that codepage's table. `Codepage.Utf8` instead encodes the whole string as
UTF-8, so a caller holding a charset label has one entry point for both cases.

A single-byte codepage can spell far less than Unicode can. A character the
selected codepage has no byte for — `世` in `windows-1252`, or any letter outside
the Greek block in `ISO-8859-7` — is rejected with `ErrInvalidFormat` (`77050003`)
rather than replaced by `?` or an HTML numeric reference. Substituting is a policy
decision this call does not make for you: catch the error and substitute yourself if
that is what your format wants. The same holds for a combining sequence such as
`e` followed by `U+0301`, which no single-byte codepage has one byte for, and for
`U+FFFD` itself.

Use `encoding::utf8Encode` when the target really is UTF-8 and no codepage
dispatch is needed."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_codepageEncode(codepage AS Codepage, text AS String) AS List OF Byte
  IF codepage = Codepage.Utf8 THEN
    RETURN __encoding_utf8Encode(text)
  END IF
  LET table AS String = __encoding_codepageTable(codepage)
  MUT out AS List OF Byte = []
  FOR EACH ch IN strings::graphemes(text)
    ' A grapheme wider than one scalar has no single-byte form at all.
    IF len(ch) <> 1 THEN
      FAIL error(77050003, "character not representable in this codepage")
    END IF
    LET point AS Integer = collections::get(__encoding_codepoints(ch), 0)
    IF point < 128 THEN
      out = collections::append(out, toByte(point))
    ELSE
      ' Reject the hole sentinel BEFORE searching, or it matches an unmapped slot
      ' and this would emit that slot's byte.
      IF ch = "\u{FFFD}" THEN
        FAIL error(77050003, "character not representable in this codepage")
      END IF
      IF NOT strings::contains(table, ch) THEN
        FAIL error(77050003, "character not representable in this codepage")
      END IF
      out = collections::append(out, toByte(strings::find(table, ch) + 128))
    END IF
  NEXT
  RETURN out
END FUNC"#;
const EX: &str = r#"Write text back out as `windows-1252` bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::codepageEncode(encoding::Codepage.Windows1252, "café")
  io::print(encoding::hexEncode(bytes))
END SUB
```

Round-trip a body through a codepage, and handle a character the codepage cannot
spell:

```
IMPORT encoding
IMPORT io

FUNC toCodepage(cp AS encoding::Codepage, text AS String) AS String
  RETURN encoding::hexEncode(encoding::codepageEncode(cp, text))
  TRAP(err)
    RETURN "unrepresentable"
  END TRAP
END FUNC

SUB main()
  LET raw AS List OF Byte = [toByte(200), toByte(233)]
  LET text AS String = encoding::codepageDecode(encoding::Codepage.Iso8859_5, raw)
  io::print(toCodepage(encoding::Codepage.Iso8859_5, text))
  io::print(toCodepage(encoding::Codepage.Windows1252, "世"))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "codepageEncode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "codepage",
                    desc: "The codepage to write the bytes in.",
                    aliases: &[],
                    ty: ParameterType::named("Codepage"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "text",
                    desc: "The text to encode. The empty string encodes to no bytes.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec!["ErrInvalidFormat"],
            body: Body::mfb(BODY, "__encoding_codepageEncode"),
        }],
    });
}
