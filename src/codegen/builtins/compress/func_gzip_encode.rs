//! `compress::gzipEncode(data, level)` — compress to one gzip (RFC 1952) member.
//!
//! Pure-MFB rewrite onto `__compress_gzipEncode` (registered by [`super::helper_gzip_encode`]),
//! which validates the level, writes the header, runs the shared encoder core and appends the
//! CRC-32 and length trailer. No native code: the same bytes and the same errors on every target.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str = r#"Compress bytes into gzip data — the format of `.gz` files and of gzip-encoded HTTP bodies."#;
const DESC: &str = r#"`compress::gzipEncode(data, level)` compresses `data` into the gzip format, the format of
`.gz` files and of the `gzip` HTTP content encoding. The result is one gzip member: a ten-byte
header, DEFLATE data, and a trailer holding a CRC-32 checksum of `data` and its length.
`compress::gzipDecode` turns the result back into `data`, and so do the `gzip` command-line tool
and every HTTP client that accepts `gzip`. For raw DEFLATE data, use `compress::deflate`; for
the zlib format, use `compress::zlibEncode`.

The header stores no file name and no modification time, and names the operating system as
unknown. So the same `data` and `level` give the same bytes wherever and whenever the program
runs.

`level` trades time for size. It runs from `0` to `9` and defaults to `6`. Level `0` stores
the data without compressing it: it is the fastest, and the result is a few bytes larger than
`data`. Levels `1` to `9` compress, searching harder for repeated runs of bytes as the level
rises, so a higher level takes longer and usually gives a smaller result. A level outside `0`
to `9` raises `ErrInvalidArgument`.

The result is valid gzip data, but it is not byte-for-byte what zlib's own compressor would
produce.

`compress::gzipEncode` is written in MFBASIC itself and uses no system library."#;
const EX: &str = r#"Compress some repetitive text into gzip data, then decompress it again:

```
IMPORT compress
IMPORT encoding
IMPORT io

SUB main()
  LET text AS List OF Byte = encoding::utf8Encode("hello, hello, hello, hello, hello, hello, hello")
  LET packed AS List OF Byte = compress::gzipEncode(text, 9)
  io::print(toString(len(text)) & " bytes compressed to " & toString(len(packed)))
  io::print(encoding::utf8Decode(compress::gzipDecode(packed)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "gzipEncode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF Byte[, Integer]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "data",
                    desc: "The bytes to compress.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "level",
                    desc: "How hard to compress, from `0` (store without compressing) to `9` (smallest result, slowest); defaults to `6`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: "6",
                    },
                },
            ],
            return_type: bytes(),
            errors: vec!["ErrInvalidArgument"],
            body: Body::Rewrite("__compress_gzipEncode"),
        }],
    });
}
