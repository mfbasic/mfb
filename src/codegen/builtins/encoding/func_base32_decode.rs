//! `encoding::base32Decode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/base32Decode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, BYTES};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Decode a standard Base32 `String` into a `List OF Byte`."#;
const DESC: &str = r#"`encoding::base32Decode` parses `text` as standard Base32 (RFC 4648 §6) and
returns the bytes it encodes. Each character selects a 5-bit value from the
alphabet `ABCDEFGHIJKLMNOPQRSTUVWXYZ234567`; the values are concatenated
most-significant bit first into a continuous bit stream and emitted eight bits at
a time, so leftover bits that do not fill a final byte are discarded. This is the
inverse of `encoding::base32Encode`.

Decoding is case-insensitive: `A`–`Z` and `a`–`z` map to the same values `0`–`25`,
and the digits `2`–`7` map to `26`–`31`. The `=` character is treated as padding
and may appear only as a trailing run; once a `=` is seen, any later non-padding
character is rejected. Padding characters are otherwise ignored and do not
contribute bits.

The total input length (including padding) must be a multiple of eight
characters. In addition, the number of non-padding symbols must correspond to a
valid Base32 group boundary: a symbol count whose remainder modulo eight is `1`,
`3`, or `6` cannot occur in any well-formed Base32 encoding and is rejected. The
empty string decodes to the empty list."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_base32Decode(text AS String) AS List OF Byte
  LET data AS List OF Byte = strings::toBytes(text)
  LET total AS Integer = len(data)
  IF total - (total / 8) * 8 <> 0 THEN
    FAIL error(77050003, "invalid base32 length")
  END IF
  MUT values AS List OF Integer = []
  MUT i AS Integer = 0
  MUT seenPad AS Boolean = FALSE
  MUT c AS Integer = 0
  MUT v AS Integer = 0
  WHILE i < total
    c = toInt(collections::get(data, i))
    IF c = 61 THEN
      seenPad = TRUE
    ELSE
      IF seenPad THEN
        FAIL error(77050003, "invalid base32 padding")
      END IF
      v = __encoding_base32Value(c)
      IF v < 0 THEN
        FAIL error(77050003, "invalid base32 character")
      END IF
      values = collections::append(values, v)
    END IF
    i = i + 1
  END WHILE
  LET symbols AS Integer = len(values)
  LET tail AS Integer = symbols - (symbols / 8) * 8
  IF tail = 1 OR tail = 3 OR tail = 6 THEN
    FAIL error(77050003, "invalid base32 length")
  END IF
  RETURN __encoding_baseDecodeBits(values, 5)
END FUNC"#;
const EX: &str = r#"Decode a Base32 string back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::base32Decode("NBSWY3DP")
  io::print(encoding::utf8Decode(bytes))
END SUB
```

Round-trip through `base32Encode`:

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

pub(crate) const BASE32_DECODE: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.base32Decode",
    "base32Decode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("text", &[], "String")], BYTES)],
    BODY,
)
.with_example(EX);
