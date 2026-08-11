//! `encoding::utf8Decode` — descriptor entry, docs, selector, and both variants.
//!
//! Per-member file (mirrors collections/func_*.rs). `utf8Decode` is an
//! `Implementation::Resolve` compile-time overload: [`resolve`] picks the byte- or
//! integer-typed decoder from the argument's element type, and both candidates live
//! inline in [`VARIANTS`] as **private** `Implementation::Mfb` bodies — they are not
//! separately-registered public functions, only monomorph targets of `utf8Decode`.
//! Authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/utf8Decode.md`. Bodies byte-significant (2-space
//! indent → `.ncode` columns); do not reformat.

use super::{ov, p, BYTES, INTS};
use crate::codegen::registry::{BuiltinFunction, Implementation, Variant};

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

// The two private variant bodies (`FUNC __encoding_*`), injected into the package
// source at their `'@@MFB_BODY:<slug>@@` markers. Byte-significant; do not reformat.
#[rustfmt::skip]
const BODY_BYTES: &str =
r#"FUNC __encoding_utf8DecodeBytes(value AS List OF Byte) AS String
  IF __encoding_utf8Valid(value) = FALSE THEN
    FAIL error(77050003, "invalid utf-8")
  END IF
  RETURN toString(value)
END FUNC"#;
#[rustfmt::skip]
const BODY_INTS: &str =
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

/// The compile-time overload selector for `utf8Decode` (an
/// [`crate::codegen::registry::ResolveFn`]): a `List OF Byte` argument picks the
/// byte-typed decoder, a `List OF Integer` picks the integer-typed one; the return
/// type is `String` either way, so the contextual expected type is unused. Any
/// other argument shape is not this overload (`Ok(None)`).
fn resolve(arg_types: &[String], _expected: Option<&str>) -> Result<Option<&'static str>, ()> {
    if arg_types == [BYTES] {
        Ok(Some("encoding.utf8DecodeBytes"))
    } else if arg_types == [INTS] {
        Ok(Some("encoding.utf8DecodeInts"))
    } else {
        Ok(None)
    }
}

/// The private candidates [`resolve`] chooses among — each its own internal
/// `Mfb` body, injected under its slug and reached only as a monomorph target.
const VARIANTS: &[Variant] = &[
    Variant {
        name: "encoding.utf8DecodeBytes",
        doc_slug: "utf8DecodeBytes",
        return_type: "String",
        implementation: Implementation::Mfb {
            body: BODY_BYTES,
            fast_path: None,
        },
    },
    Variant {
        name: "encoding.utf8DecodeInts",
        doc_slug: "utf8DecodeInts",
        return_type: "String",
        implementation: Implementation::Mfb {
            body: BODY_INTS,
            fast_path: None,
        },
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
