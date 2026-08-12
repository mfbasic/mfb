//! `encoding::percentDecode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/percentDecode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, VALTEXT};
use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str = r#"Decode a percent-encoded (URL-encoded) `String` back into text."#;
const DESC: &str = r#"`encoding::percentDecode` reverses `encoding::percentEncode`, expanding every
`%XX` escape in `text` back into the byte it names. The input is scanned as its
raw byte sequence: each `%` (byte 37) introduces a two-digit hexadecimal escape
whose value becomes a single output byte, and every other byte is copied through
unchanged. The accumulated bytes are then interpreted as UTF-8 to produce the
returned `String`.

The two hex digits after a `%` accept either case (`0`–`9`, `a`–`f`, `A`–`F`) and
may be mixed. Unlike `encoding::formUrlDecode`, a literal `+` (byte 43) is *not*
translated to a space — it passes through verbatim — because plus-as-space is an
`application/x-www-form-urlencoded` convention, not part of RFC 3986 percent
encoding.

The empty string decodes to the empty string. The function is a strict decoder:
a `%` with fewer than two following bytes, a `%` followed by a non-hex digit, or
a decoded byte sequence that is not valid UTF-8 all raise an error rather than
being passed through or replaced."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_percentDecode(text AS String) AS String
  RETURN __encoding_percentDecodeBytes(text, FALSE)
END FUNC"#;
const EX: &str = r#"Decode a percent-encoded string containing a space escape:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::percentDecode("a%20b"))
END SUB
```

Round-trip through `percentEncode`, including a non-ASCII character:

```
IMPORT encoding
IMPORT io

SUB main()
  LET enc AS String = encoding::percentEncode("café & tea")
  io::print(enc)
  io::print(encoding::percentDecode(enc))
END SUB
```"#;

pub(crate) const PERCENT_DECODE: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.percentDecode",
    "percentDecode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("value", VALTEXT, "String")], "String")],
    BODY,
)
.with_example(EX);
