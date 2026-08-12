//! `encoding::utf32Decode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/utf32Decode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, INTS};
use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str = r#"Decode a `List OF Integer` of UTF-32 code points to a `String`."#;
const DESC: &str = r#"`encoding::utf32Decode` interprets `value` as a sequence of UTF-32 code points
and returns the corresponding text. Each element is a full Unicode scalar value:
because UTF-32 is a fixed-width encoding, one list element decodes directly to
one scalar, with no multi-unit sequences or surrogate pairs to combine. The empty
list decodes to the empty string.

Every element must be a valid Unicode scalar. A code point is rejected when it is
negative or greater than `1114111` (`0x10FFFF`), or when it lies in the surrogate
range `55296..57343` (`0xD800..0xDFFF`) — surrogates are not scalar values and
cannot appear on their own in UTF-32. Any such element fails rather than
producing replacement text. The elements are treated as numeric code points, not
a byte serialization, so no byte order (endianness) or byte-order mark applies.


`utf32Decode` is the inverse of `encoding::utf32Encode`: decoding the code points
that `utf32Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_utf32Decode(value AS List OF Integer) AS String
  MUT out AS String = ""
  FOR EACH cp IN value
    IF cp < 0 OR cp > 1114111 THEN
      FAIL error(77050003, "invalid code point")
    END IF
    IF cp >= 55296 AND cp <= 57343 THEN
      FAIL error(77050003, "surrogate code point")
    END IF
    out = out & __encoding_fromCodepoint(cp)
  NEXT
  RETURN out
END FUNC"#;
const EX: &str = r#"Decode UTF-32 code points back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::utf32Decode([104, 105]))
END SUB
```

Round-trip an astral scalar (an emoji) through UTF-32:

```
IMPORT encoding
IMPORT io

SUB main()
  LET points AS List OF Integer = encoding::utf32Encode("😀")
  io::print(encoding::utf32Decode(points))
END SUB
```"#;

pub(crate) const UTF32_DECODE: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.utf32Decode",
    "utf32Decode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("value", &[], INTS)], "String")],
    BODY,
)
.with_example(EX);
