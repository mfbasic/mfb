//! `encoding::uleb128Encode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/uleb128Encode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction, RegistryPackage,
};

const INTRO: &str = r#"Encode a non-negative `Integer` as an unsigned LEB128 `List OF Byte`."#;
const DESC: &str = r#"`encoding::uleb128Encode` returns the unsigned [LEB128](https://en.wikipedia.org/wiki/LEB128)
representation of `value`, a base-128 little-endian variable-length encoding.
The value is split into 7-bit groups, least-significant group first. Each output
byte carries one group in its low seven bits; the high bit (`0x80`) is set on
every byte except the last, where it is clear, marking the end of the sequence.


At least one byte is always emitted: `0` encodes as the single byte `[0]`.
Because groups are produced until the remaining value reaches zero, the output
length grows by one byte for every additional seven significant bits — for
example values in `0`–`127` produce one byte, `128`–`16383` produce two bytes,
and so on.

`value` must be non-negative; unsigned LEB128 has no representation for negative
numbers. Use `encoding::sleb128Encode` for signed values. The inverse operation
is `encoding::uleb128Decode`, which reads one unsigned LEB128 sequence back into
an `Integer`."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_uleb128Encode(value AS Integer) AS List OF Byte
  IF value < 0 THEN
    FAIL error(77050003, "negative value")
  END IF
  RETURN __encoding_leb128Emit(value)
END FUNC"#;
const EX: &str = r#"Encode a value and round-trip it back through `uleb128Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::uleb128Encode(624485)
  io::print(toString(encoding::uleb128Decode(bytes)))
END SUB
```

Small values fit in a single byte:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(toString(len(encoding::uleb128Encode(0))))
  io::print(toString(len(encoding::uleb128Encode(127))))
  io::print(toString(len(encoding::uleb128Encode(128))))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "uleb128Encode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The non-negative integer to encode.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: Body::mfb(BODY, "__encoding_uleb128Encode"),
        }],
    });
}
