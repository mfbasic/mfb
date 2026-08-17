//! `crypto::hmacSha512` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). A two-overload SOURCE member: the
//! `List OF Byte` and `String` `data` forms are distinct `Implementation`s whose
//! parameter type makes `select` pick the `_bytes`/`_text` body in `package.mfb`
//! (the legacy `_bytes`/`_text` `implementation_name` the clean-room `select()`
//! subsumes). Docs migrated from `src/docs/man/builtins/crypto/hmacSha512.md`.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str =
    r#"Compute the HMAC-SHA-512 message authentication code (RFC 2104) of a message under a key."#;
const DESC: &str = r#"`crypto::hmacSha512` computes the keyed-hash message authentication code of
`data` under `key`, using SHA-512 as the underlying hash, as specified by
RFC 2104. It returns a fixed 64-byte (512-bit) MAC as a `List OF Byte`.

Keys of any length are accepted. Per RFC 2104, a key longer than the 128-byte
SHA-512 block size is first hashed down to 64 bytes, and any key shorter than
the block size is right-padded with zero bytes to 128 bytes before the inner and
outer passes.

The MAC is a deterministic function of `key` and `data` alone: the same key and
message always produce the same 64 bytes, with no salting or randomness. The
function is **total** — every combination of inputs, including empty key and
empty message, yields a MAC and it never raises an error.

The MAC is a portable software core computed over the `bits` package, so its
output is **byte-identical on every target** (macOS/Linux, aarch64/x86-64) and
uses no platform crypto library. A MAC is raw binary, not text; stringify it with
`encoding::hexEncode` or `encoding::base64Encode` to display or store it. To
compare a received MAC against a computed one, use `crypto::constantTimeEqual` so
the comparison does not leak timing information. The `List OF Byte` overload
authenticates the raw bytes as given; the `String` overload authenticates the
string's UTF-8 encoding."#;
const EX: &str = r#"Authenticate a message and print the MAC as hex:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET mac AS List OF Byte = crypto::hmacSha512(key, message)
  io::print(encoding::hexEncode(mac))
END SUB
```

Verify a received MAC in constant time:

```
IMPORT crypto
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET received AS List OF Byte = crypto::hmacSha512(key, "payload")
  LET expected AS List OF Byte = crypto::hmacSha512(key, "payload")
  IF crypto::constantTimeEqual(expected, received) THEN
    io::print("authentic")
  END IF
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hmacSha512",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF Byte, (List OF Byte or String)"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "key",
                        desc: "The secret HMAC key.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "data",
                        desc: "The message bytes to authenticate.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                ],
                return_type: bytes(),
                errors: vec![],
                body: Body::Rewrite("__crypto_hmacSha512_bytes"),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "key",
                        desc: "The secret HMAC key.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "data",
                        desc: "A string whose UTF-8 bytes are authenticated.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                ],
                return_type: bytes(),
                errors: vec![],
                body: Body::Rewrite("__crypto_hmacSha512_text"),
            },
        ],
    });
}
