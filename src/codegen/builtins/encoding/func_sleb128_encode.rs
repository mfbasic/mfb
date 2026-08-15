//! `encoding::sleb128Encode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/sleb128Encode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction, RegistryPackage,
};

const INTRO: &str = r#"Encode a signed `Integer` as a signed LEB128 `List OF Byte`."#;
const DESC: &str = r#"`encoding::sleb128Encode` returns the signed [LEB128](https://en.wikipedia.org/wiki/LEB128)
representation of `value`, a base-128 little-endian variable-length encoding
that carries the sign. The value is split into 7-bit groups, least-significant
group first. Each output byte holds one group in its low seven bits; the high
bit (`0x80`) is set on every byte except the last, where it is clear, marking
the end of the sequence.

Unlike unsigned LEB128, encoding continues by arithmetic (sign-extending) shift
rather than logical shift: after each group `value` is shifted right by seven
bits with the sign preserved. The sequence terminates only when the remaining
bits are all sign bits *and* the sign bit of the final group (`0x40`) matches —
that is, when the remaining value is `0` and the group's sign bit is clear, or
the remaining value is `-1` and the group's sign bit is set. This guarantees the
top byte sign-extends correctly on decode.

At least one byte is always emitted: `0` encodes as the single byte `[0]` and
`-1` encodes as the single byte `[0x7F]`. Both non-negative and negative values
are accepted; use `encoding::uleb128Encode` when the value is known to be
non-negative and the sign byte is unwanted. The inverse operation is
`encoding::sleb128Decode`, which reads one signed LEB128 sequence back into an
`Integer`."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_sleb128Encode(value AS Integer) AS List OF Byte
  MUT result AS List OF Byte = []
  MUT remaining AS Integer = value
  MUT chunk AS Integer = 0
  MUT more AS Boolean = TRUE
  MUT signBit AS Integer = 0
  WHILE more
    chunk = bits::band(remaining, 127)
    remaining = bits::sra(remaining, 7)
    signBit = bits::band(chunk, 64)
    IF remaining = 0 AND signBit = 0 THEN
      more = FALSE
    ELSE
      IF remaining = -1 AND signBit <> 0 THEN
        more = FALSE
      END IF
    END IF
    IF more THEN
      result = collections::append(result, toByte(chunk + 128))
    ELSE
      result = collections::append(result, toByte(chunk))
    END IF
  END WHILE
  RETURN result
END FUNC"#;
const EX: &str = r#"Encode a value and round-trip it back through `sleb128Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::sleb128Encode(-123456)
  io::print(toString(encoding::sleb128Decode(bytes)))
END SUB
```

Small values fit in a single byte:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(toString(len(encoding::sleb128Encode(0))))
  io::print(toString(len(encoding::sleb128Encode(-1))))
  io::print(toString(len(encoding::sleb128Encode(-64))))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sleb128Encode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The integer to encode.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: Body::mfb(BODY, "__encoding_sleb128Encode"),
        }],
    });
}
