//! `compress::zlibEncode(data, level)` — compress to zlib (RFC 1950) data.
//!
//! Pure-MFB rewrite onto `__compress_zlibEncode` (registered by [`super::helper_zlib_encode`]),
//! which validates the level, writes the header, runs the shared encoder core and appends the
//! Adler-32 trailer. No native code: the same bytes and the same errors on every target.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Compress bytes into zlib data — DEFLATE data with a two-byte header and an Adler-32 checksum, so it can be checked on arrival."#;
const DESC: &str = r#"`compress::zlibEncode(data, level)` compresses `data` into the zlib format: DEFLATE data
with a two-byte header in front and an Adler-32 checksum of `data` behind, so whoever decodes it
can tell whether it arrived intact. It is what HTTP calls the `deflate` content encoding, and what
a PNG image stores. `compress::zlibDecode` turns the result back into `data`. For raw DEFLATE
data with no header, use `compress::deflate`; for gzip, use `compress::gzipEncode`.

`level` trades time for size. It runs from `0` to `9` and defaults to `6`. Level `0` stores
the data without compressing it: it is the fastest, and the result is a few bytes larger than
`data`. Levels `1` to `9` compress, searching harder for repeated runs of bytes as the level
rises, so a higher level takes longer and usually gives a smaller result. The header records
which range the level was in, as zlib does; decoders do not need it. A level outside `0` to `9`
raises `ErrInvalidArgument`.

The result is valid zlib data, so zlib and every tool built on it can decompress it. It is not
byte-for-byte what zlib's own compressor would produce. The same `data` and `level` always give
the same bytes, on every platform.

`compress::zlibEncode` is written in MFBASIC itself and uses no system library."#;
const EX: &str = r#"Compress some repetitive text into zlib data, then decompress it again:

```
IMPORT compress
IMPORT encoding
IMPORT io

SUB main()
  LET text AS List OF Byte = encoding::utf8Encode("hello, hello, hello, hello, hello, hello, hello")
  LET packed AS List OF Byte = compress::zlibEncode(text)
  io::print(toString(len(text)) & " bytes compressed to " & toString(len(packed)))
  io::print(encoding::utf8Decode(compress::zlibDecode(packed)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "zlibEncode",
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
            body: Body::Rewrite("__compress_zlibEncode"),
        }],
    });
}
