//! `compress::zlibDecode(data, maxBytes, ignoreChecksum)` — decompress zlib (RFC 1950) data.
//!
//! Pure-MFB rewrite onto `__compress_zlibDecode` (registered by [`super::helper_zlib_frame`]), which
//! checks the zlib header, runs the shared decoder core and verifies the Adler-32 trailer.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str = r#"Decompress zlib data — DEFLATE data with a two-byte header and an Adler-32 checksum — checking that it arrived intact."#;
const DESC: &str = r#"`compress::zlibDecode(data, maxBytes, ignoreChecksum)` decompresses `data` in the zlib
format and returns the original bytes. The zlib format wraps DEFLATE data in a two-byte header
and ends it with an Adler-32 checksum of the decompressed bytes; it is what HTTP calls the
`deflate` content encoding, and what a PNG image stores. For raw DEFLATE data with no header,
use `compress::inflate`; for gzip data, use `compress::gzipDecode`.

`maxBytes` caps how large the result may be, and defaults to `67108864` (64 MiB). If the data
would decompress to more than `maxBytes` bytes, `zlibDecode` raises `ErrTooLarge` as soon as it
reaches the limit. A result of exactly `maxBytes` bytes is allowed. A negative `maxBytes` raises
`ErrInvalidArgument`.

`zlibDecode` raises `ErrInvalidFormat` when the header is not a zlib header, when the header asks
for a preset dictionary (which this package does not support), when the DEFLATE data is
malformed, when the data ends early, and when the checksum does not match the decompressed bytes.
With `ignoreChecksum` set to `TRUE` the checksum is not compared, so data whose checksum was
damaged still decompresses; the four checksum bytes must still be there, and every other check
still applies. Bytes after the checksum are ignored.

`compress::zlibDecode` is written in MFBASIC itself and uses no system library, so it returns
the same bytes and raises the same errors on every platform."#;
const EX: &str = r#"Decompress zlib data and print the text inside it:

```
IMPORT compress
IMPORT encoding
IMPORT io

SUB main()
  LET packed AS List OF Byte = encoding::hexDecode("789ccb48cdc9c9d751c840a21492f3730b8a528b8b0191f40a61")
  io::print(encoding::utf8Decode(compress::zlibDecode(packed)))
END SUB
```

A damaged checksum is refused unless you ask for it to be skipped:

```
IMPORT collections
IMPORT compress
IMPORT encoding
IMPORT io

SUB main()
  MUT packed AS List OF Byte = encoding::hexDecode("789ccb48cdc9c9d751c840a21492f3730b8a528b8b0191f40a61")
  packed = collections::set(packed, len(packed) - 1, toByte(0))
  LET strict AS List OF Byte = compress::zlibDecode(packed) TRAP(e)
    io::print("refused: " & e.message)
    RECOVER []
  END TRAP
  LET lenient AS List OF Byte = compress::zlibDecode(packed, 67108864, TRUE)
  io::print(encoding::utf8Decode(lenient))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "zlibDecode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF Byte[, Integer[, Boolean]]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "data",
                    desc: "zlib data: a two-byte header, DEFLATE data, then a four-byte Adler-32 checksum. Bytes after the checksum are ignored.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "maxBytes",
                    desc: "The largest result allowed, in bytes; defaults to `67108864` (64 MiB). Larger output raises `ErrTooLarge`. Must not be negative.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: "67108864",
                    },
                },
                Parameter {
                    name: "ignoreChecksum",
                    desc: "`TRUE` skips comparing the Adler-32 checksum; the checksum bytes must still be present. Defaults to `FALSE`.",
                    aliases: &[],
                    ty: ParameterType::Boolean,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Boolean,
                        expr: "false",
                    },
                },
            ],
            return_type: bytes(),
            errors: vec!["ErrInvalidFormat", "ErrTooLarge", "ErrInvalidArgument"],
            body: Body::Rewrite("__compress_zlibDecode"),
        }],
    });
}
