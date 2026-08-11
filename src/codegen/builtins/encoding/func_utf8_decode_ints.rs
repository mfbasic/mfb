//! `encoding::utf8DecodeInts` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/utf8DecodeInts.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ef, ov, p, INTS};
use crate::codegen::registry::{BuiltinFunction, Implementation};

const INTRO: &str = r#"Decode a `List OF Integer` of UTF-8 code units to a `String`."#;
const DESC: &str = r#"`encoding::utf8DecodeInts` interprets `value` as a UTF-8 byte sequence held one
octet per integer element and returns the corresponding text. Each element is
first range-checked and narrowed to a byte: every unit must lie in `0..255`
(0 through 255 inclusive), and the assembled bytes must form a well-formed UTF-8
sequence, with no invalid, overlong, or truncated byte sequence. If both checks
pass, the octets become the string's storage. The empty list decodes to the
empty string.

This is the integer-typed form of `encoding::utf8Decode`. `utf8Decode` is a
parameter overload that selects between a `List OF Byte` and a `List OF Integer`
argument at compile time; `utf8DecodeInts` is the concrete, non-overloaded name
that always takes a `List OF Integer`, so no overload resolution is involved.
The byte-typed counterpart is `encoding::utf8DecodeBytes`, which takes a
`List OF Byte` and therefore performs no per-element range check.


It is the inverse of `encoding::utf8EncodeInts`: decoding the integers that
`utf8EncodeInts` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_utf8DecodeInts(value AS List OF Integer) AS String
  MUT data AS List OF Byte = []
  FOR EACH unit IN value
    IF unit < 0 OR unit > 255 THEN
      FAIL error(77050003, "invalid utf-8 code unit")
    END IF
    data = collections::append(data, toByte(unit))
  NEXT
  IF __encoding_utf8Valid(data) = FALSE THEN
    FAIL error(77050003, "invalid utf-8")
  END IF
  RETURN toString(data)
END FUNC"#;
const EX: &str = r#"Decode UTF-8 code units back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8EncodeInts("héllo")
  io::print(encoding::utf8DecodeInts(units))
END SUB
```

Round-trip a string through its UTF-8 code units:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8EncodeInts("hi")
  io::print(encoding::utf8DecodeInts(units))
END SUB
```"#;

pub(crate) const UTF8_DECODE_INTS: BuiltinFunction = ef(
    "encoding.utf8DecodeInts",
    "utf8DecodeInts",
    &[ov(&[p("value", &[], INTS)], "String")],
    Implementation::Mfb {
        body: BODY,
        fast_path: None,
    },
)
.with_intro(INTRO)
.with_desc(DESC)
.with_example(EX);
