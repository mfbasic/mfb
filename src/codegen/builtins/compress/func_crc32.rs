//! `compress::crc32(data, running)` — the CRC-32 checksum gzip and PNG use.
//!
//! Pure-MFB rewrite onto `__compress_crc32` (registered by
//! [`super::helper_crc32`]), a slicing-by-8 loop over the 2,048-entry table in
//! [`super::helper_crc32_table`]. No native code: the same bytes and the same errors
//! on every target.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str = r#"Compute the CRC-32 checksum of a list of bytes, optionally continuing from an earlier checksum."#;
const DESC: &str = r#"`compress::crc32(data, running)` returns the CRC-32 checksum of `data` as an
`Integer` from `0` to `4294967295`. It is the standard CRC-32 used by gzip, zip,
PNG and Ethernet (catalogued as CRC-32/ISO-HDLC), so the value matches what
those formats store and what other languages' zlib `crc32` returns: the
checksum of the nine bytes of `"123456789"` is `3421780262` (`0xCBF43926`).

`running` continues a checksum. Pass the result of an earlier call and the new
bytes are checked as though they followed the earlier ones, so data that arrives
in pieces never has to be joined first: `crc32(b, crc32(a))` equals the checksum
of `a` followed by `b`. `running` defaults to `0`, which starts a new checksum,
and `crc32([], running)` returns `running` unchanged.

`running` must be a value `crc32` can return — `0` to `4294967295`. Anything
outside that range raises `ErrInvalidArgument`.

A CRC-32 detects accidental corruption: a flipped bit, a truncated download, a
damaged block. It is not a security check — anyone can change data and fix up
its CRC-32 to match. Use `crypto::hash` or `crypto::hmac` when the data might
have been altered on purpose.

`compress::crc32` is written in MFBASIC itself and uses no system library, so it
returns the same value on every platform."#;
const EX: &str = r#"Print the checksum of the standard check string:

```
IMPORT compress
IMPORT io
IMPORT strings

SUB main()
  io::print(toString(compress::crc32(strings::toBytes("123456789"))))
END SUB
```

Check data that arrives in two pieces without joining them:

```
IMPORT compress
IMPORT io
IMPORT strings

SUB main()
  LET first AS List OF Byte = strings::toBytes("12345")
  LET second AS List OF Byte = strings::toBytes("6789")
  LET running AS Integer = compress::crc32(first)
  io::print(toString(compress::crc32(second, running)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "crc32",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF Byte[, Integer]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "data",
                    desc: "The bytes to check. Any length is accepted, including the empty list.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "running",
                    desc: "A checksum returned by an earlier call, to continue from; `0` (the default) starts a new checksum. Must be `0` to `4294967295`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: "0",
                    },
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec!["ErrInvalidArgument"],
            body: Body::Rewrite("__compress_crc32"),
        }],
    });
}
