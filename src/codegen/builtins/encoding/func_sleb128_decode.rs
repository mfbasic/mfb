//! `encoding::sleb128Decode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/sleb128Decode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, BYTES};
use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str = r#"Decode a signed LEB128 `List OF Byte` back into an `Integer`."#;
const DESC: &str = r#"`encoding::sleb128Decode` reads one signed [LEB128](https://en.wikipedia.org/wiki/LEB128)
sequence from `data` and returns the `Integer` it represents. It is the inverse
of `encoding::sleb128Encode`.

Bytes are consumed least-significant group first. The low seven bits of each
byte contribute the next 7-bit group; the high bit (`0x80`) is the continuation
flag. Decoding accumulates groups — shifting each successive group left by seven
more bits — and stops at the first byte whose high bit is clear (byte value
below `128`), which terminates the sequence. Any bytes after that terminator are
ignored.

Unlike `encoding::uleb128Decode`, the terminating group carries the sign. When
the final byte's sign bit (`0x40`) is set and the accumulated shift is still
below `64`, the result is sign-extended by filling every higher bit with ones, so
the value decodes as negative. A clear `0x40` leaves the value non-negative. This
mirrors the arithmetic (sign-extending) shift used by `encoding::sleb128Encode`.


`data` must contain at least one byte, and the sequence must be terminated
within it: if the bytes run out before a byte with a clear high bit is seen, the
input is treated as truncated. The accumulated shift may not exceed `63` bits;
a sequence encoding more than 64 significant bits overflows."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_sleb128Decode(data AS List OF Byte) AS Integer
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
      IF shift < 64 AND bits::band(byteValue, 64) <> 0 THEN
        result = bits::bor(result, bits::sl(-1, shift))
      END IF
    END IF
  END WHILE
  RETURN result
END FUNC"#;
const EX: &str = r#"Round-trip a signed value through `sleb128Encode` and back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::sleb128Encode(-123456)
  io::print(toString(encoding::sleb128Decode(bytes)))
END SUB
```

Decode a single terminating byte whose `0x40` sign bit is set (`-2` = `[0x7E]`):

```
IMPORT encoding
IMPORT collections
IMPORT io

SUB main()
  MUT bytes AS List OF Byte = []
  bytes = collections::append(bytes, toByte(126))
  io::print(toString(encoding::sleb128Decode(bytes)))
END SUB
```"#;

pub(crate) const SLEB128_DECODE: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.sleb128Decode",
    "sleb128Decode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("data", &[], BYTES)], "Integer")],
    BODY,
)
.with_example(EX);
