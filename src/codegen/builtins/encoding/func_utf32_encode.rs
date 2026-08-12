//! `encoding::utf32Encode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/utf32Encode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, INTS, VALTEXT};
use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str = r#"Encode a `String` to its UTF-32 code points."#;
const DESC: &str = r#"`encoding::utf32Encode` returns the UTF-32 encoding of `value` as a list of
numeric code points, one element per Unicode scalar value. Each scalar is a
number in the range `0..1114111` (`0x10FFFF`); because a valid `String` holds no
surrogate scalars, the result never contains a value in the surrogate range
`55296..57343`.

The scalars are produced by decoding the string's UTF-8 bytes in order: each
1-to-4-byte sequence contributes exactly one code point, so the returned list
has one element per Unicode scalar in `value` (which may be fewer than its byte
length).

These are UTF-32 *code points*, not a byte serialization: the result is a
sequence of numbers, so no byte order (endianness) or byte-order mark applies.
The function is **total** — every string, including the empty string (which
yields an empty list), encodes successfully, and it never raises a runtime
error. The inverse operation is `encoding::utf32Decode`, which turns a
`List OF Integer` of code points back into a `String` and rejects out-of-range
or surrogate code points."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_utf32Encode(value AS String) AS List OF Integer
  RETURN __encoding_codepoints(value)
END FUNC"#;
const EX: &str = r#"Encode a string to its UTF-32 code points:

```
IMPORT encoding
IMPORT io

SUB main()
  LET points AS List OF Integer = encoding::utf32Encode("hello")
  io::print(toString(len(points)))
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

pub(crate) const UTF32_ENCODE: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.utf32Encode",
    "utf32Encode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("value", VALTEXT, "String")], INTS)],
    BODY,
)
.with_example(EX);
