//! `encoding::percentEncode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/percentEncode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, VALTEXT};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Percent-encode (URL-encode) a `String` per RFC 3986."#;
const DESC: &str = r#"`encoding::percentEncode` percent-encodes `text` following the RFC 3986 rules for
the *unreserved* character set. The input is first converted to its UTF-8 byte
sequence, then each byte is emitted in order.

A byte passes through unchanged when it is a member of the unreserved set:
the ASCII letters `A`–`Z` (65–90) and `a`–`z` (97–122), the digits `0`–`9`
(48–57), and the four marks `-` (45), `.` (46), `_` (95), and `~` (126). Every
other byte — including space, reserved and sub-delimiter characters, control
bytes, and every continuation byte of a multi-byte UTF-8 character — is emitted
as a three-character escape `%XX`, where `XX` is the byte value in **uppercase**
hexadecimal.

Because non-ASCII characters are encoded from their UTF-8 bytes, a single such
character expands to one `%XX` escape per byte (two escapes for most Latin and
symbol characters, three or four for higher code points). The function is
**total**: every `String`, including the empty string (which yields the empty
string), encodes successfully and it never raises a runtime error.

The inverse operation is `encoding::percentDecode`, which parses `%XX` escapes
back into text. For `application/x-www-form-urlencoded` data, where space is
encoded as `+`, use `encoding::formUrlEncode` instead."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_percentEncode(text AS String) AS String
  LET data AS List OF Byte = strings::toBytes(text)
  MUT out AS String = ""
  MUT c AS Integer = 0
  FOR EACH b IN data
    c = toInt(b)
    IF __encoding_isUnreserved(c) THEN
      out = out & __encoding_byteChar(c)
    ELSE
      out = out & __encoding_percentByte(c)
    END IF
  NEXT
  RETURN out
END FUNC"#;
const EX: &str = r#"Encode a path segment containing reserved characters:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::percentEncode("a b/c"))
END SUB
```

Round-trip through `percentDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET enc AS String = encoding::percentEncode("café & tea")
  io::print(enc)
  io::print(encoding::percentDecode(enc))
END SUB
```"#;

pub(crate) const PERCENT_ENCODE: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.percentEncode",
    "percentEncode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("value", VALTEXT, "String")], "String")],
    BODY,
)
.with_example(EX);
