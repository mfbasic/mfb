//! `encoding::utf8Decode` — descriptor entry, docs, selector, and both variants.
//!
//! Per-member file (mirrors collections/func_*.rs). `utf8Decode` is an
//! `Implementation::Resolve` compile-time overload: [`resolve`] picks the byte- or
//! integer-typed decoder from the argument's element type, and both candidate
//! descriptors ([`UTF8_DECODE_BYTES`], [`UTF8_DECODE_INTS`]) — resolver *and* every
//! implementation — live here in this one file. Authored intro/description/examples
//! migrated from `src/docs/man/builtins/encoding/utf8Decode.md`. Bodies
//! byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, BYTES, INTS};
use crate::codegen::registry::{BuiltinFunction, Variant};

const INTRO: &str = r#"Decode a UTF-8 byte or code-unit sequence to a `String`."#;
const DESC: &str = r#"`encoding::utf8Decode` interprets `value` as a UTF-8 byte sequence and returns the
corresponding text. Because MFBASIC strings are always well-formed UTF-8, the
input is validated in full before the string is produced: `utf8Decode` accepts
only a well-formed UTF-8 sequence and rejects an invalid lead byte, a missing or
stray continuation byte, a truncated multi-byte sequence, an overlong encoding, a
surrogate code point (`U+D800`–`U+DFFF`), and any scalar above `U+10FFFF`. The
empty list decodes to the empty string.


`utf8Decode` is a **parameter overload** selected by the argument's element type:
a `List OF Byte` is decoded directly, while a `List OF Integer` is first checked
element by element — each unit must lie in `0..255` — then decoded. The overload
is resolved during monomorphization, so the selection is a compile-time decision,
not a runtime dispatch.

It is the inverse of `encoding::utf8Encode`: decoding the bytes (or integers)
that `utf8Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
const EX: &str = r#"Decode raw UTF-8 bytes back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("héllo")
  io::print(encoding::utf8Decode(raw))
END SUB
```

Decode from a `List OF Integer` code-unit list:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = [104, 105]
  io::print(encoding::utf8Decode(units))
END SUB
```"#;

/// The compile-time overload selector for `utf8Decode` (an [`crate::codegen::registry::ResolveFn`]):
/// a `List OF Byte` argument picks the byte-typed decoder, a `List OF Integer`
/// picks the integer-typed one; the return type is `String` either way, so the
/// contextual expected type is unused. Any other argument shape is not this
/// overload (`Ok(None)`).
fn resolve(arg_types: &[String], _expected: Option<&str>) -> Result<Option<&'static str>, ()> {
    if arg_types == [BYTES] {
        Ok(Some(bytes::UTF8_DECODE_BYTES.name))
    } else if arg_types == [INTS] {
        Ok(Some(ints::UTF8_DECODE_INTS.name))
    } else {
        Ok(None)
    }
}

/// The candidates [`resolve`] chooses among — each a registered sibling descriptor
/// (below) plus the return type its name yields.
const VARIANTS: &[Variant] = &[
    Variant {
        name: bytes::UTF8_DECODE_BYTES.name,
        return_type: "String",
    },
    Variant {
        name: ints::UTF8_DECODE_INTS.name,
        return_type: "String",
    },
];

pub(crate) const UTF8_DECODE: BuiltinFunction = BuiltinFunction::resolve(
    "encoding.utf8Decode",
    "utf8Decode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("value", &[], BYTES)], "String")],
    resolve,
    VARIANTS,
)
.with_example(EX);

pub(crate) use bytes::UTF8_DECODE_BYTES;
pub(crate) use ints::UTF8_DECODE_INTS;

/// The byte-typed decoder (`List OF Byte`), one candidate of [`resolve`].
mod bytes {
    use super::super::{ov, p, BYTES};
    use crate::codegen::registry::BuiltinFunction;

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

    pub(crate) const UTF8_DECODE_BYTES: BuiltinFunction = BuiltinFunction::mfb(
        "encoding.utf8DecodeBytes",
        "utf8DecodeBytes",
        INTRO,
        DESC,
        &[],
        &[ov(&[p("value", &[], BYTES)], "String")],
        BODY,
    )
    .with_example(EX);
}

/// The integer-typed decoder (`List OF Integer`), one candidate of [`resolve`].
mod ints {
    use super::super::{ov, p, INTS};
    use crate::codegen::registry::BuiltinFunction;

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

    pub(crate) const UTF8_DECODE_INTS: BuiltinFunction = BuiltinFunction::mfb(
        "encoding.utf8DecodeInts",
        "utf8DecodeInts",
        INTRO,
        DESC,
        &[],
        &[ov(&[p("value", &[], INTS)], "String")],
        BODY,
    )
    .with_example(EX);
}
