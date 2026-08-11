//! `encoding::utf16Decode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/utf16Decode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, INTS};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Decode a `List OF Integer` of UTF-16 code units to a `String`."#;
const DESC: &str = r#"`encoding::utf16Decode` interprets `value` as a sequence of UTF-16 code units and
returns the corresponding text. Each element is examined in order: a unit in the
Basic Multilingual Plane decodes to a single Unicode scalar, while a high
surrogate in `55296..56319` is combined with the following low surrogate in
`56320..57343` to reconstruct one astral scalar. The empty list decodes to the
empty string.

A surrogate pair is recombined by subtracting the surrogate offsets, shifting the
high unit up by ten bits, adding the low ten bits, and adding `65536`, yielding a
scalar above `65535`.

Every element must lie in `0..65535`; a value outside that range is rejected. A
high surrogate that is the last element, or is followed by a unit that is not a
low surrogate, is an unpaired surrogate, as is a low surrogate that does not
follow a high surrogate — all of these fail rather than producing replacement
text. The units are treated as numeric code units, not a byte serialization, so
no byte order (endianness) or byte-order mark applies.


`utf16Decode` is the inverse of `encoding::utf16Encode`: decoding the code units
that `utf16Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_utf16Decode(value AS List OF Integer) AS String
  LET n AS Integer = len(value)
  MUT out AS String = ""
  MUT i AS Integer = 0
  MUT unit AS Integer = 0
  MUT low AS Integer = 0
  MUT scalar AS Integer = 0
  WHILE i < n
    unit = collections::get(value, i)
    IF unit < 0 OR unit > 65535 THEN
      FAIL error(77050003, "invalid utf-16 code unit")
    END IF
    IF unit >= 55296 AND unit <= 56319 THEN
      IF i + 1 >= n THEN
        FAIL error(77050003, "unpaired surrogate")
      END IF
      low = collections::get(value, i + 1)
      IF low < 56320 OR low > 57343 THEN
        FAIL error(77050003, "unpaired surrogate")
      END IF
      scalar = 65536 + bits::sl(unit - 55296, 10) + (low - 56320)
      out = out & __encoding_fromCodepoint(scalar)
      i = i + 2
    ELSE
      IF unit >= 56320 AND unit <= 57343 THEN
        FAIL error(77050003, "unpaired surrogate")
      END IF
      out = out & __encoding_fromCodepoint(unit)
      i = i + 1
    END IF
  END WHILE
  RETURN out
END FUNC"#;
const EX: &str = r#"Decode UTF-16 code units back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::utf16Decode([104, 105]))
END SUB
```

Round-trip an astral scalar (an emoji) through UTF-16:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf16Encode("😀")
  io::print(encoding::utf16Decode(units))
END SUB
```"#;

pub(crate) const UTF16_DECODE: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.utf16Decode",
    "utf16Decode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("value", &[], INTS)], "String")],
    BODY,
)
.with_example(EX);
