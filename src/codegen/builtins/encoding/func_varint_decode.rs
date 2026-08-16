//! `encoding::varintDecode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/varintDecode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Decode a ZigZag varint `List OF Byte` back into a signed `Integer`."#;
const DESC: &str = r#"`encoding::varintDecode` reads one ZigZag [varint](https://protobuf.dev/programming-guides/encoding/#varints)
sequence from `data` and returns the signed `Integer` it represents. It is the
inverse of `encoding::varintEncode`.

Decoding proceeds in two steps. First the bytes are read as an unsigned
[LEB128](https://en.wikipedia.org/wiki/LEB128) sequence — least-significant 7-bit
group first, with the high bit (`0x80`) of each byte marking continuation and the
first byte with a clear high bit terminating the sequence. Then the ZigZag
mapping is reversed — `(u >> 1) XOR -(u AND 1)` — turning the unsigned value back
into the original signed value, so that small-magnitude negatives round-trip
correctly. Because the ZigZag reversal is pure arithmetic on the decoded value,
it never fails on its own; every error surfaces from the underlying LEB128 read.


`data` must contain at least one byte, and the sequence must be terminated within
it: if the bytes run out before a byte with a clear high bit is seen, the input
is treated as truncated. The accumulated shift may not exceed 63 bits; a sequence
encoding more than 64 significant bits overflows. Any bytes after the terminator
are ignored."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_varintDecode(data AS List OF Byte) AS Integer
  LET zigzag AS Integer = __encoding_uleb128Decode(data)
  RETURN bits::bxor(bits::sr(zigzag, 1), 0 - bits::band(zigzag, 1))
END FUNC"#;
const EX: &str = r#"Round-trip a signed value through `varintEncode` and back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::varintEncode(-75)
  io::print(toString(encoding::varintDecode(bytes)))
END SUB
```

Decode a literal two-byte sequence (`-75` = `[0x95, 0x01]`):

```
IMPORT encoding
IMPORT collections
IMPORT io

SUB main()
  MUT bytes AS List OF Byte = []
  bytes = collections::append(bytes, toByte(149))
  bytes = collections::append(bytes, toByte(1))
  io::print(toString(encoding::varintDecode(bytes)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "varintDecode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "data",
                desc: "The varint bytes to decode.",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::Byte),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::mfb(BODY, "__encoding_varintDecode"),
        }],
    });
}
