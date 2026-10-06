//! `encoding::codepageEncode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors the other `encoding` codecs). The descriptor carries an
//! MFBASIC source body. Body byte-significant (2-space indent → `.ncode` columns);
//! do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Encode a `String` as bytes in a legacy single-byte codepage or as UTF-8."#;
const DESC: &str = r#"`encoding::codepageEncode` writes `text` as bytes in `codepage`, and is the exact
inverse of `encoding::codepageDecode`: for any single-byte codepage, bytes that
decode without failing re-encode to exactly those bytes.

Each codepage is one of the `encoding::Codepage` members. For every single-byte
codepage, a character below `U+0080` is its own byte, and anything above is looked
up in that codepage's table. `Codepage.Utf8` instead encodes the whole string as
UTF-8, so a caller holding a charset label has one entry point for both cases.

A single-byte codepage can spell far less than Unicode can. A character the
selected codepage has no byte for — `世` in `windows-1252`, or a non-Greek letter
above `U+007F` in `ISO-8859-7` — is rejected with `ErrInvalidFormat` (`77050003`)
rather than replaced by `?` or an HTML numeric reference. Substituting is a policy
decision this call does not make for you: catch the error and substitute yourself if
that is what your format wants. `U+FFFD` itself is always rejected, even for a
codepage whose table happens to hold a byte at that slot's index.

A codepage maps Unicode **scalars**, not graphemes: `text` is encoded one scalar
at a time, so a multi-scalar grapheme — `"\r\n"` (one grapheme, two scalars in
this runtime), or a base letter followed by a combining mark some codepages spell
as their own byte (Windows-1258's combining acute, Windows-1255's Hebrew points,
ISO-8859-6's Arabic harakat) — encodes successfully whenever every one of its
scalars is in the table, as however many bytes it has scalars. Only a scalar the
table has no slot for (or `U+FFFD`) raises; it does not matter whether that
scalar is standalone or part of a larger grapheme.

Use `encoding::utf8Encode` when the target really is UTF-8 and no codepage
dispatch is needed. `ErrOutOfMemory` means the program ran out of memory."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_codepageEncode(codepage AS Codepage, text AS String) AS List OF Byte
  IF codepage = Codepage.Utf8 THEN
    RETURN __encoding_utf8Encode(text)
  END IF
  LET table AS String = __encoding_codepageTable(codepage)
  MUT out AS List OF Byte = []
  ' bug-861: a codepage maps SCALARS, not graphemes. Walking `strings::graphemes`
  ' and rejecting anything wider than one scalar rejected "\r\n" (one grapheme,
  ' two scalars, in this runtime) and a base letter plus a combining mark the
  ' codepage spells as its own byte (Windows-1258's combining acute, Windows-1255's
  ' Hebrew points, ISO-8859-6's Arabic harakat) -- breaking the decode/encode round
  ' trip for exactly the content those codepages carry. Walking scalars instead
  ' encodes each one independently, which is what the table actually indexes.
  FOR EACH point IN __encoding_codepoints(text)
    IF point < 128 THEN
      out = collections::append(out, toByte(point))
    ELSE
      ' Reject the hole sentinel BEFORE searching, or it matches an unmapped slot
      ' and this would emit that slot's byte. U+FFFD = 65533.
      IF point = 65533 THEN
        FAIL error(77050003, "character not representable in this codepage")
      END IF
      LET ch AS String = __encoding_fromCodepoint(point)
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

FUNC main() AS Nothing
  LET bytes AS List OF Byte = encoding::codepageEncode(encoding::Codepage.Windows1252, "café")
  io::print(encoding::hexEncode(bytes))
END FUNC
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

FUNC main() AS Nothing
  LET raw AS List OF Byte = [toByte(200), toByte(233)]
  LET text AS String = encoding::codepageDecode(encoding::Codepage.Iso8859_5, raw)
  io::print(toCodepage(encoding::Codepage.Iso8859_5, text))
  io::print(toCodepage(encoding::Codepage.Windows1252, "世"))
END FUNC
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
                    desc: "The codepage to write the bytes in. `Codepage.Utf8` encodes the whole text as UTF-8 instead.",
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
            errors: vec!["ErrInvalidFormat", "ErrOutOfMemory"],
            body: Body::mfb(BODY, "__encoding_codepageEncode"),
        }],
    });
}
