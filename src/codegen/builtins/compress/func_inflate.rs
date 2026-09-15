//! `compress::inflate(data, maxBytes)` — decompress raw DEFLATE (RFC 1951) data.
//!
//! Pure-MFB rewrite onto `__compress_inflate` (registered by [`super::helper_inflate`]), which
//! runs the shared decoder core ([`super::helper_inflate_core`]) and removes its end-position
//! trailer. No native code: the same bytes and the same errors on every target.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str = r#"Decompress raw DEFLATE data — the compressed format inside zlib, gzip, zip and PNG — refusing anything malformed."#;
const DESC: &str = r#"`compress::inflate(data, maxBytes)` decompresses `data`, which must be raw DEFLATE
data with no header or trailer around it, and returns the original bytes. Use it when a
format stores DEFLATE data directly, as zip entries do. For data that starts with a zlib
header, use `compress::zlibDecode`; for a `.gz` file or a gzip-encoded response, use
`compress::gzipDecode`.

`maxBytes` caps how large the result may be, and defaults to `67108864` (64 MiB). If the data
would decompress to more than `maxBytes` bytes, `inflate` raises `ErrTooLarge` as soon as it
reaches the limit, so a small hostile input cannot make the program build a huge result. A
result of exactly `maxBytes` bytes is allowed. `maxBytes` must not be negative; a negative
value raises `ErrInvalidArgument`.

Data that is not valid DEFLATE raises `ErrInvalidFormat`: an invalid block, a code table that
does not describe a proper Huffman code, a reference to bytes before the start of the output,
or data that ends before its final block does. `inflate` accepts exactly what zlib's own
decoder accepts, so data produced by any conforming compressor decodes, and data that zlib
refuses is refused here too. Bytes after the end of the final block are ignored.

`compress::inflate` is written in MFBASIC itself and uses no system library, so it returns
the same bytes and raises the same errors on every platform."#;
const EX: &str = r#"Decompress a short raw DEFLATE stream and print the text inside it:

```
IMPORT compress
IMPORT encoding
IMPORT io

SUB main()
  LET packed AS List OF Byte = encoding::hexDecode("cb48cdc9c9d751c840a21492f3730b8a528b8b01")
  LET unpacked AS List OF Byte = compress::inflate(packed)
  io::print(encoding::utf8Decode(unpacked))
END SUB
```

Refuse data that would decompress to more than you are prepared to handle:

```
IMPORT compress
IMPORT encoding
IMPORT io

SUB main()
  LET packed AS List OF Byte = encoding::hexDecode("cb48cdc9c9d751c840a21492f3730b8a528b8b01")
  LET unpacked AS List OF Byte = compress::inflate(packed, 10) TRAP(e)
    io::print("refused: " & e.message)
    EXIT SUB
  END TRAP
  io::print(toString(len(unpacked)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "inflate",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF Byte[, Integer]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "data",
                    desc: "Raw DEFLATE data, with no zlib or gzip header. Bytes after the final block are ignored.",
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
            ],
            return_type: bytes(),
            errors: vec!["ErrInvalidFormat", "ErrTooLarge", "ErrInvalidArgument"],
            body: Body::Rewrite("__compress_inflate"),
        }],
    });
}
