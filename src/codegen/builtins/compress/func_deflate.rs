//! `compress::deflate(data, level)` — compress to raw DEFLATE (RFC 1951) data.
//!
//! Pure-MFB rewrite onto `__compress_deflate` (registered by [`super::helper_deflate`]), which
//! validates the level and runs the shared encoder core ([`super::helper_deflate_core`]). No
//! native code: the same bytes and the same errors on every target.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Compress bytes into raw DEFLATE data — the compressed format inside zlib, gzip, zip and PNG."#;
const DESC: &str = r#"`compress::deflate(data, level)` compresses `data` and returns raw DEFLATE data, with no
header or trailer around it. Use it when a format stores DEFLATE data directly, as zip entries
do. For the zlib format, use `compress::zlibEncode`; for gzip, use `compress::gzipEncode`.
`compress::inflate` turns the result back into `data`.

`level` trades time for size. It runs from `0` to `9` and defaults to `6`. Level `0` stores
the data without compressing it: it is the fastest, and the result is a few bytes larger than
`data`. Levels `1` to `9` compress, searching harder for repeated runs of bytes as the level
rises, so a higher level takes longer and usually gives a smaller result. A level outside `0`
to `9` raises `ErrInvalidArgument`.

The result is valid DEFLATE data, so zlib and every tool built on it can decompress it. It is
not byte-for-byte what zlib's own compressor would produce. The same `data` and `level` always
give the same bytes, on every platform.

`compress::deflate` is written in MFBASIC itself and uses no system library."#;
const EX: &str = r#"Compress some repetitive text, then decompress it again:

```
IMPORT compress
IMPORT encoding
IMPORT io

SUB main()
  LET text AS List OF Byte = encoding::utf8Encode("hello, hello, hello, hello, hello, hello, hello")
  LET packed AS List OF Byte = compress::deflate(text)
  io::print(toString(len(text)) & " bytes compressed to " & toString(len(packed)))
  io::print(encoding::utf8Decode(compress::inflate(packed)))
END SUB
```

Level `0` stores the bytes; level `9` searches hardest:

```
IMPORT compress
IMPORT encoding
IMPORT io

SUB main()
  LET text AS List OF Byte = encoding::utf8Encode("abcabcabcabcabcabcabcabcabcabcabcabc")
  io::print("level 0: " & toString(len(compress::deflate(text, 0))) & " bytes")
  io::print("level 9: " & toString(len(compress::deflate(text, 9))) & " bytes")
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "deflate",
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
            body: Body::Rewrite("__compress_deflate"),
        }],
    });
}
