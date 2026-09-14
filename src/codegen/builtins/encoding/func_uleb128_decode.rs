//! `encoding::uleb128Decode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Decode an unsigned LEB128 `List OF Byte` back into an `Integer`."#;
const DESC: &str = r#"`encoding::uleb128Decode` reads one unsigned [LEB128](https://en.wikipedia.org/wiki/LEB128)
sequence from `data` and returns the `Integer` it represents. It is the inverse
of `encoding::uleb128Encode`.

Bytes are read least-significant group first. The low seven bits of each
byte contribute the next 7-bit group; the high bit (`0x80`) is the continuation
flag. Decoding accumulates groups — shifting each successive group left by seven
more bits — and stops at the first byte whose high bit is clear (byte value
below `128`), which terminates the sequence. Any bytes after that terminator are
ignored.

`data` must contain at least one byte, and the sequence must be terminated
within it: an empty list, or bytes that run out before a byte with a clear high
bit is seen, raise `ErrInvalidFormat`, as does a sequence longer than ten bytes.
Keep values within `9223372036854775807`: a tenth byte that reaches bit 63 or
beyond is not currently rejected, so it can decode to a negative number or lose
its high bits (nine `0xFF` bytes then `0x01` decode to `-1`). Use
`encoding::sleb128Decode` for signed values."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_uleb128Decode(data AS List OF Byte) AS Integer
  LET n AS Integer = len(data)
  IF n = 0 THEN
    FAIL error(77050003, "truncated leb128")
  END IF
  MUT result AS Integer = 0
  MUT shift AS Integer = 0
  MUT i AS Integer = 0
  MUT byteValue AS Integer = 0
  MUT done AS Boolean = FALSE
  WHILE done = FALSE
    IF i >= n THEN
      FAIL error(77050003, "truncated leb128")
    END IF
    IF shift > 63 THEN
      FAIL error(77050003, "leb128 overflow")
    END IF
    byteValue = toInt(collections::get(data, i))
    result = bits::bor(result, bits::sl(bits::band(byteValue, 127), shift))
    shift = shift + 7
    i = i + 1
    IF byteValue < 128 THEN
      done = TRUE
    END IF
  END WHILE
  RETURN result
END FUNC"#;
const EX: &str = r#"Round-trip a value through `uleb128Encode` and back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::uleb128Encode(624485)
  io::print(toString(encoding::uleb128Decode(bytes)))
END SUB
```

Decode a literal two-byte sequence (`300` = `[0xAC, 0x02]`):

```
IMPORT encoding
IMPORT collections
IMPORT io

SUB main()
  MUT bytes AS List OF Byte = []
  bytes = collections::append(bytes, toByte(172))
  bytes = collections::append(bytes, toByte(2))
  io::print(toString(encoding::uleb128Decode(bytes)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "uleb128Decode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "data",
                desc: "The ULEB128 bytes to decode: a non-empty, terminated sequence. Bytes after the terminating byte are ignored.",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::Byte),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::mfb(BODY, "__encoding_uleb128Decode"),
        }],
    });
}
