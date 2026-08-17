//! `encoding::utf16Encode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Encode a `String` to its UTF-16 code units."#;
const DESC: &str = r#"`encoding::utf16Encode` returns the UTF-16 encoding of `value` as a list of
numeric code units, one element per 16-bit unit. Each Unicode scalar in `value`
is examined in order: a scalar in the Basic Multilingual Plane (`0..65535`)
becomes a single code unit, and an astral scalar (above `65535`) is split into a
surrogate pair — a high surrogate in `55296..56319` followed by a low surrogate
in `56320..57343`.

The surrogate split subtracts `65536` from the scalar, then takes the top ten
bits (offset by `55296`) as the high unit and the low ten bits (offset by
`56320`) as the low unit, so every returned element lies in `0..65535`.


These are UTF-16 *code units*, not a byte serialization: the result is a
sequence of numbers, so no byte order (endianness) or byte-order mark applies.
The function is **total** — every string, including the empty string (which
yields an empty list), encodes successfully, and it never raises a runtime
error. The inverse operation is `encoding::utf16Decode`, which turns a
`List OF Integer` of code units back into a `String` and rejects unpaired
surrogates and out-of-range units."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_utf16Encode(value AS String) AS List OF Integer
  LET points AS List OF Integer = __encoding_codepoints(value)
  MUT result AS List OF Integer = []
  MUT scalar AS Integer = 0
  MUT high AS Integer = 0
  MUT low AS Integer = 0
  FOR EACH cp IN points
    IF cp <= 65535 THEN
      result = collections::append(result, cp)
    ELSE
      scalar = cp - 65536
      high = 55296 + bits::sr(scalar, 10)
      low = 56320 + bits::band(scalar, 1023)
      result = collections::append(result, high)
      result = collections::append(result, low)
    END IF
  NEXT
  RETURN result
END FUNC"#;
const EX: &str = r#"Encode a string to its UTF-16 code units:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf16Encode("hello")
  io::print(toString(len(units)))
END SUB
```

Round-trip an astral scalar (an emoji) through UTF-16:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf16Encode("😀")
  io::print(encoding::utf16Decode(units))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "utf16Encode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string to encode.",
                aliases: &["text"],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Integer),
            errors: vec![],
            body: Body::mfb(BODY, "__encoding_utf16Encode"),
        }],
    });
}
