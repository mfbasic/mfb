//! `encoding::utf8EncodeBytes` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/utf8EncodeBytes.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ef, ov, p, BYTES};
use crate::codegen::registry::{BuiltinFunction, Implementation};

const INTRO: &str = r#"Encode a `String` to its UTF-8 bytes as a `List OF Byte`."#;
const DESC: &str = r#"`encoding::utf8EncodeBytes` returns the UTF-8 encoding of `value` — the exact
bytes that make up the string's storage — as a `List OF Byte`, one element per
byte. Because MFBASIC strings are always UTF-8 text, the result is the string's
raw octets in order, each element in the range `0..255`. The result is exactly
`strings::toBytes(value)`.

This is the byte-typed form of `encoding::utf8Encode`. `utf8Encode` is a
return-type overload that selects between `List OF Byte` and `List OF Integer`
from the call's contextual type; `utf8EncodeBytes` is the concrete, non-overloaded
name that always yields `List OF Byte`, so no type context is needed to
disambiguate it. The integer-typed counterpart is `encoding::utf8EncodeInts`.


The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error.

The inverse operation is `encoding::utf8DecodeBytes`, which accepts a
`List OF Byte` and validates it as well-formed UTF-8."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_utf8EncodeBytes(value AS String) AS List OF Byte
  RETURN strings::toBytes(value)
END FUNC"#;
const EX: &str = r#"Encode a string to raw UTF-8 bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8EncodeBytes("héllo")
  io::print(toString(len(raw)))
END SUB
```

Round-trip a string through its UTF-8 bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8EncodeBytes("hi")
  io::print(encoding::utf8DecodeBytes(raw))
END SUB
```"#;

pub(crate) const UTF8_ENCODE_BYTES: BuiltinFunction = ef(
    "encoding.utf8EncodeBytes",
    "utf8EncodeBytes",
    &[ov(&[p("value", &[], "String")], BYTES)],
    Implementation::Mfb {
        body: BODY,
        fast_path: None,
    },
)
.with_intro(INTRO)
.with_desc(DESC)
.with_example(EX);
