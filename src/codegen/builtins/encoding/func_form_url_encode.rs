//! `encoding::formUrlEncode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/formUrlEncode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, VALTEXT};
use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str = r#"Encode a `String` as `application/x-www-form-urlencoded` data."#;
const DESC: &str = r#"`encoding::formUrlEncode` encodes `text` using the
`application/x-www-form-urlencoded` rules that HTML forms apply to query-string
values. The input is first converted to its UTF-8 byte sequence, then each byte
is emitted in order.

A byte passes through unchanged only when it is an ASCII alphanumeric: the
letters `A`–`Z` (65–90) and `a`–`z` (97–122) and the digits `0`–`9` (48–57).
The space byte (32) is emitted as a single `+`. Every other byte — including
`-`, `.`, `_`, `~`, reserved and sub-delimiter characters, control bytes, and
every continuation byte of a multi-byte UTF-8 character — is emitted as a
three-character escape `%XX`, where `XX` is the byte value in **uppercase**
hexadecimal.

This differs from `encoding::percentEncode`, which leaves the four unreserved
marks `-`, `.`, `_`, and `~` untouched and escapes space as `%20` rather than
`+`. Because non-ASCII characters are encoded from their UTF-8 bytes, a single
such character expands to one `%XX` escape per byte (two escapes for most Latin
and symbol characters, three or four for higher code points).

The function is **total**: every `String`, including the empty string (which
yields the empty string), encodes successfully and it never raises a runtime
error. The inverse operation is `encoding::formUrlDecode`, which parses `%XX`
escapes and `+` back into text."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_formUrlEncode(text AS String) AS String
  LET data AS List OF Byte = strings::toBytes(text)
  MUT out AS String = ""
  MUT c AS Integer = 0
  FOR EACH b IN data
    c = toInt(b)
    IF __encoding_isAlphaNum(c) THEN
      out = out & __encoding_byteChar(c)
    ELSE
      IF c = 32 THEN
        out = out & "+"
      ELSE
        out = out & __encoding_percentByte(c)
      END IF
    END IF
  NEXT
  RETURN out
END FUNC"#;
const EX: &str = r#"Encode a form field value containing a space and reserved characters:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::formUrlEncode("name = a b & c"))
END SUB
```

Round-trip through `formUrlDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET enc AS String = encoding::formUrlEncode("café & tea")
  io::print(enc)
  io::print(encoding::formUrlDecode(enc))
END SUB
```"#;

pub(crate) const FORM_URL_ENCODE: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.formUrlEncode",
    "formUrlEncode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("value", VALTEXT, "String")], "String")],
    BODY,
)
.with_example(EX);
