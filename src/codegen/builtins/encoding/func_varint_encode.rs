//! `encoding::varintEncode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/varintEncode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, RegistryFunction,
    RegistryPackage,
};

const INTRO: &str = r#"Encode a signed `Integer` as a ZigZag varint `List OF Byte`."#;
const DESC: &str = r#"`encoding::varintEncode` returns the ZigZag [varint](https://protobuf.dev/programming-guides/encoding/#varints)
representation of `value`. It first maps the signed value onto an unsigned one
with ZigZag encoding — `(value << 1) XOR (value >> 63)`, an arithmetic
right shift — so that small-magnitude negatives become small unsigned numbers,
then writes that unsigned result as base-128 [LEB128](https://en.wikipedia.org/wiki/LEB128).


The ZigZag mapping interleaves signs: `0` maps to `0`, `-1` to `1`, `1` to `2`,
`-2` to `3`, and so on. The mapped value is then split into 7-bit groups,
least-significant group first. Each output byte carries one group in its low
seven bits; the high bit (`0x80`) is set on every byte except the last, where it
is clear, marking the end of the sequence. Because the intermediate value is
shifted right logically, encoding always terminates and at least one byte is
always emitted: `0` encodes as the single byte `[0]`.


Unlike `encoding::uleb128Encode`, `value` may be negative — ZigZag gives every
signed value a compact unsigned form, so no value is rejected. The inverse
operation is `encoding::varintDecode`, which reads one ZigZag varint sequence
back into a signed `Integer`."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_varintEncode(value AS Integer) AS List OF Byte
  LET zigzag AS Integer = bits::bxor(bits::sl(value, 1), bits::sra(value, 63))
  RETURN __encoding_leb128Emit(zigzag)
END FUNC"#;
const EX: &str = r#"Encode a signed value and round-trip it back through `varintDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::varintEncode(-75)
  io::print(toString(encoding::varintDecode(bytes)))
END SUB
```

Small-magnitude values, positive or negative, fit in a single byte:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(toString(len(encoding::varintEncode(0))))
  io::print(toString(len(encoding::varintEncode(-1))))
  io::print(toString(len(encoding::varintEncode(63))))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "varintEncode",
        intro: INTRO,
        desc: DESC,
        example: EX,
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
            lowering: Lowering::Helper,
            body: Body::mfb(BODY, "__encoding_varintEncode"),
        }],
    });
}
