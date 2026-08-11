//! `encoding::utf8EncodeInts` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/utf8EncodeInts.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, INTS};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Encode a `String` to its UTF-8 bytes as a `List OF Integer`."#;
const DESC: &str = r#"`encoding::utf8EncodeInts` returns the UTF-8 encoding of `value` — the exact
bytes that make up the string's storage — as a `List OF Integer`, one element per
byte. Because MFBASIC strings are always UTF-8 text, the result is the string's
raw octets in order, each element widened to `Integer` and in the range `0..255`.
The elements are exactly the values of `strings::toBytes(value)` converted with
`toInt`.

This is the integer-typed form of `encoding::utf8Encode`. `utf8Encode` is a
return-type overload that selects between `List OF Byte` and `List OF Integer`
from the call's contextual type; `utf8EncodeInts` is the concrete, non-overloaded
name that always yields `List OF Integer`, so no type context is needed to
disambiguate it. The byte-typed counterpart is `encoding::utf8EncodeBytes`.


The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error.

The inverse operation is `encoding::utf8DecodeInts`, which accepts a
`List OF Integer` and validates it as well-formed UTF-8."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_utf8EncodeInts(value AS String) AS List OF Integer
  LET data AS List OF Byte = strings::toBytes(value)
  MUT result AS List OF Integer = []
  FOR EACH b IN data
    result = collections::append(result, toInt(b))
  NEXT
  RETURN result
END FUNC"#;
const EX: &str = r#"Encode a string to its UTF-8 code units:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8EncodeInts("héllo")
  io::print(toString(len(units)))
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

pub(crate) const UTF8_ENCODE_INTS: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.utf8EncodeInts",
    "utf8EncodeInts",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("value", &[], "String")], INTS)],
    BODY,
)
.with_example(EX);
