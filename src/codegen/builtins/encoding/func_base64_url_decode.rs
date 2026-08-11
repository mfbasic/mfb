//! `encoding::base64UrlDecode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/base64UrlDecode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, BYTES};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Decode a URL- and filename-safe Base64 `String` into a `List OF Byte`."#;
const DESC: &str = r#"`encoding::base64UrlDecode` parses `text` as URL- and filename-safe Base64
(RFC 4648 §5) and returns the bytes it encodes. Each character selects a 6-bit
value from the alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_`; the values
are concatenated most-significant bit first into a continuous bit stream and
emitted eight bits at a time, so leftover bits that do not fill a final byte are
discarded. This is the inverse of `encoding::base64UrlEncode`.

The alphabet is the URL-safe variant using `-` and `_` for values `62` and `63`;
it is case-sensitive (`A`–`Z` map to `0`–`25`, `a`–`z` to `26`–`51`, `0`–`9` to
`52`–`61`). The `=` character is treated as padding: once a `=` is seen, any
later non-padding character is rejected. Padding characters are otherwise ignored
and contribute no bits.

Unlike `encoding::base64Decode`, this function does **not** require the total
input length to be a multiple of four, so URL-safe text produced without `=`
padding decodes directly; text that does carry `=` padding is also accepted. The
only length constraint is that the number of non-padding symbols cannot be
exactly one more than a multiple of four (a symbol count whose remainder modulo
four is `1`), because no well-formed Base64 group ends on a single 6-bit symbol.
The empty string decodes to the empty list. For the standard variant that uses
`+` and `/`, use `encoding::base64Decode`."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_base64UrlDecode(text AS String) AS List OF Byte
  LET values AS List OF Integer = __encoding_base64Symbols(text, TRUE)
  LET symbols AS Integer = len(values)
  IF symbols - (symbols / 4) * 4 = 1 THEN
    FAIL error(77050003, "invalid base64 length")
  END IF
  RETURN __encoding_baseDecodeBits(values, 6)
END FUNC"#;
const EX: &str = r#"Decode a URL-safe Base64 string (no padding) back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::base64UrlDecode("aGVsbG8")
  io::print(encoding::utf8Decode(bytes))
END SUB
```

Round-trip through `base64UrlEncode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base64UrlEncode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base64UrlDecode(text)))
END SUB
```"#;

pub(crate) const BASE64_URL_DECODE: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.base64UrlDecode",
    "base64UrlDecode",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("text", &[], "String")], BYTES)],
    BODY,
)
.with_example(EX);
