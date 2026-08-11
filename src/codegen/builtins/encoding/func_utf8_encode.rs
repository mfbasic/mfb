//! `encoding::utf8Encode` — descriptor entry, docs, selector, and both variants.
//!
//! Per-member file (mirrors collections/func_*.rs). `utf8Encode` is an
//! `Implementation::Resolve` **return-type** overload: [`resolve`] picks the byte-
//! or integer-typed encoder from the expected (contextual) type, erroring
//! (`Err(())` → `TYPE_OVERLOAD_AMBIGUOUS`) when no context selects one — a
//! `LET … AS List OF Byte/Integer` disambiguates. Both candidates live inline in
//! [`VARIANTS`] as **private** `Implementation::Mfb` bodies; they are not public
//! functions, only monomorph targets of `utf8Encode`. Authored docs migrated from
//! `src/docs/man/builtins/encoding/utf8Encode.md`. Bodies byte-significant (2-space
//! indent → `.ncode` columns); do not reformat.

use super::{ov, p, BYTES, INTS, VALTEXT};
use crate::codegen::registry::{BuiltinFunction, Implementation, Variant};

const INTRO: &str = r#"Encode a `String` to its UTF-8 bytes."#;
const DESC: &str = r#"`encoding::utf8Encode` returns the UTF-8 encoding of `value` — the exact bytes
that make up the string's storage — one element per byte. Because MFBASIC strings
are always UTF-8 text, the result is the string's raw octets in order, with each
element in the range `0..255`.

The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error. The
byte form is exactly `strings::toBytes(value)`; the integer form contains the
identical numeric values widened to `Integer`.

`utf8Encode` is a **return-type overload**: the same `String` argument produces
either a `List OF Byte` or a `List OF Integer`, chosen by the expected
(contextual) type at the call site. A call with no type context to select the
overload is a compile-time `TYPE_OVERLOAD_AMBIGUOUS` error, not a runtime failure;
the overload is resolved during monomorphization.

The inverse operation is `encoding::utf8Decode`, which accepts either a
`List OF Byte` or a `List OF Integer` and validates it as well-formed UTF-8."#;
const EX: &str = r#"Encode a string to raw UTF-8 bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("héllo")
  io::print(toString(len(raw)))
END SUB
```

Encode to the `List OF Integer` form and round-trip it back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8Encode("hi")
  io::print(encoding::utf8Decode(units))
END SUB
```"#;

// The two private variant bodies (`FUNC __encoding_*`), injected into the package
// source at their `'@@MFB_BODY:<slug>@@` markers. Byte-significant; do not reformat.
#[rustfmt::skip]
const BODY_BYTES: &str =
r#"FUNC __encoding_utf8EncodeBytes(value AS String) AS List OF Byte
  RETURN strings::toBytes(value)
END FUNC"#;
#[rustfmt::skip]
const BODY_INTS: &str =
r#"FUNC __encoding_utf8EncodeInts(value AS String) AS List OF Integer
  LET data AS List OF Byte = strings::toBytes(value)
  MUT result AS List OF Integer = []
  FOR EACH b IN data
    result = collections::append(result, toInt(b))
  NEXT
  RETURN result
END FUNC"#;

/// The compile-time overload selector for `utf8Encode` (an
/// [`crate::codegen::registry::ResolveFn`]): the same `String` argument encodes to
/// a `List OF Byte` or a `List OF Integer`, chosen by the expected (contextual)
/// type. With no expected type the overload is ambiguous (`Err(())` →
/// `TYPE_OVERLOAD_AMBIGUOUS`); any other argument shape is not this overload
/// (`Ok(None)`).
fn resolve(arg_types: &[String], expected: Option<&str>) -> Result<Option<&'static str>, ()> {
    if arg_types == ["String"] {
        match expected {
            Some(BYTES) => Ok(Some("encoding.utf8EncodeBytes")),
            Some(INTS) => Ok(Some("encoding.utf8EncodeInts")),
            _ => Err(()),
        }
    } else {
        Ok(None)
    }
}

/// The private candidates [`resolve`] chooses among — each its own internal
/// `Mfb` body, injected under its slug and reached only as a monomorph target.
const VARIANTS: &[Variant] = &[
    Variant {
        name: "encoding.utf8EncodeBytes",
        doc_slug: "utf8EncodeBytes",
        return_type: BYTES,
        implementation: Implementation::Mfb {
            body: BODY_BYTES,
            fast_path: None,
        },
    },
    Variant {
        name: "encoding.utf8EncodeInts",
        doc_slug: "utf8EncodeInts",
        return_type: INTS,
        implementation: Implementation::Mfb {
            body: BODY_INTS,
            fast_path: None,
        },
    },
];

pub(crate) const UTF8_ENCODE: BuiltinFunction = BuiltinFunction::resolve(
    "encoding.utf8Encode",
    "utf8Encode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("value", VALTEXT, "String")], BYTES)],
    resolve,
    VARIANTS,
)
.with_example(EX);
