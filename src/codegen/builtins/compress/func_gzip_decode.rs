//! `compress::gzipDecode(data, maxBytes, ignoreChecksum)` — decompress gzip (RFC 1952) data.
//!
//! Pure-MFB rewrite onto `__compress_gzipDecode` (registered by [`super::helper_gzip_frame`]), which
//! reads every member's header, runs the shared decoder core and verifies each CRC-32 and length.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str = r#"Decompress gzip data — a `.gz` file or a gzip-encoded response — checking every member's checksum and length."#;
const DESC: &str = r#"`compress::gzipDecode(data, maxBytes, ignoreChecksum)` decompresses `data` in the gzip
format and returns the original bytes. A gzip file is one or more members, each a header, DEFLATE
data, a CRC-32 checksum and the decompressed length; `gzipDecode` decompresses every member and
returns their contents joined in order, which is how concatenated `.gz` files are read. For zlib
data use `compress::zlibDecode`, and for raw DEFLATE data `compress::inflate`.

`maxBytes` caps how large the whole result may be, across all members, and defaults to `67108864`
(64 MiB). If the data would decompress to more than `maxBytes` bytes, `gzipDecode` raises
`ErrTooLarge` as soon as it reaches the limit. A result of exactly `maxBytes` bytes is allowed. A
negative `maxBytes` raises `ErrInvalidArgument`.

`gzipDecode` raises `ErrInvalidFormat` when the data does not start with a gzip header, when a
header uses reserved flags or runs past the end of the data, when a member's DEFLATE data is
malformed or ends early, and when a checksum or length does not match. With `ignoreChecksum` set to
`TRUE`, no checksum or length is compared — the member CRC-32s, the lengths, and a header checksum
if the header carries one — so data with damaged checksums still decompresses; those bytes must
still be present, and every other check still applies.

After the last member, another member is read only if the next two bytes are the gzip signature
`1f 8b`. Anything else after the last member is ignored, so padding written after a `.gz` file does
no harm; bytes that do start with the signature but are not a valid member are refused.

`compress::gzipDecode` is written in MFBASIC itself and uses no system library, so it returns the
same bytes and raises the same errors on every platform."#;
const EX: &str = r#"Decompress gzip data and print the text inside it:

```
IMPORT compress
IMPORT encoding
IMPORT io

SUB main()
  LET packed AS List OF Byte = encoding::hexDecode("1f8b08000000000000ffcb48cdc9c9d751c840a21492f3730b8a528b8b018d6147741c000000")
  io::print(encoding::utf8Decode(compress::gzipDecode(packed)))
END SUB
```

Two gzip members decode to their contents joined together:

```
IMPORT collections
IMPORT compress
IMPORT encoding
IMPORT io

SUB main()
  LET one AS List OF Byte = encoding::hexDecode("1f8b08000000000000ffcb48cdc9c9d751c840a21492f3730b8a528b8b018d6147741c000000")
  MUT both AS List OF Byte = one
  MUT i AS Integer = 0
  WHILE i < len(one)
    both = collections::append(both, collections::get(one, i))
    i = i + 1
  END WHILE
  io::print(toString(len(compress::gzipDecode(both))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "gzipDecode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF Byte[, Integer[, Boolean]]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "data",
                    desc: "gzip data: one or more members. After the last member, bytes that do not start with `1f 8b` are ignored.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "maxBytes",
                    desc: "The largest result allowed across all members, in bytes; defaults to `67108864` (64 MiB). Larger output raises `ErrTooLarge`. Must not be negative.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: "67108864",
                    },
                },
                Parameter {
                    name: "ignoreChecksum",
                    desc: "`TRUE` skips comparing every CRC-32, length and header checksum; those bytes must still be present. Defaults to `FALSE`.",
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
            body: Body::Rewrite("__compress_gzipDecode"),
        }],
    });
}
