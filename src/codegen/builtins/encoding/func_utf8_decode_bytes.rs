//! `encoding::utf8DecodeBytes` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/utf8DecodeBytes.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ef, ov, p, BYTES};
use crate::codegen::registry::{BuiltinFunction, Implementation};

const INTRO: &str = r#"Decode a `List OF Byte` of UTF-8 octets to a `String`."#;
const DESC: &str = r#"`encoding::utf8DecodeBytes` interprets `value` as a UTF-8 byte sequence and
returns the corresponding text. Because MFBASIC strings are always well-formed
UTF-8, the input is validated in full before the string is produced: the bytes
must form a well-formed UTF-8 sequence, with no invalid, overlong, or truncated
byte sequence. If validation succeeds, the octets are returned verbatim as the
string's storage. The empty list decodes to the empty string.


This is the byte-typed form of `encoding::utf8Decode`. `utf8Decode` is a
parameter overload that selects between a `List OF Byte` and a `List OF Integer`
argument at compile time; `utf8DecodeBytes` is the concrete, non-overloaded name
that always takes a `List OF Byte`, so no overload resolution is involved. The
integer-typed counterpart is `encoding::utf8DecodeInts`, which additionally
requires every element to be in `0..255` before decoding.


It is the inverse of `encoding::utf8EncodeBytes`: decoding the bytes that
`utf8EncodeBytes` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_utf8DecodeBytes(value AS List OF Byte) AS String
  IF __encoding_utf8Valid(value) = FALSE THEN
    FAIL error(77050003, "invalid utf-8")
  END IF
  RETURN toString(value)
END FUNC"#;
const EX: &str = r#"Decode raw UTF-8 bytes back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8EncodeBytes("héllo")
  io::print(encoding::utf8DecodeBytes(raw))
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

pub(crate) const UTF8_DECODE_BYTES: BuiltinFunction = ef(
    "encoding.utf8DecodeBytes",
    "utf8DecodeBytes",
    &[ov(&[p("value", &[], BYTES)], "String")],
    Implementation::Mfb {
        body: BODY,
        fast_path: None,
    },
)
.with_intro(INTRO)
.with_desc(DESC)
.with_example(EX);
