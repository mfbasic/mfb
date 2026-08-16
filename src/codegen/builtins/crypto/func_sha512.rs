//! `crypto::sha512` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). A two-overload SOURCE member: the
//! `List OF Byte` and `String` forms are distinct `Implementation`s whose parameter
//! type makes `select` pick the `_bytes`/`_text` body in `package.mfb` (the legacy
//! `_bytes`/`_text` `implementation_name` the clean-room `select()` subsumes). Docs
//! migrated from `src/docs/man/builtins/crypto/sha512.md`.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str = r#"Compute the SHA-512 cryptographic hash (FIPS 180-4) of a message."#;
const DESC: &str = r#"`crypto::sha512` computes the SHA-512 message digest of `data`, as specified by
FIPS 180-4, and returns it as a fixed 64-byte (512-bit) `List OF Byte`. It is one
of the four `crypto` hashes — `sha224`, `sha256`, `sha384`, and `sha512` — and is
the 512-bit member of the SHA-2 family. The implementation drives the shared
SHA-2 (64-bit) core with the SHA-512 initialization vector and a 64-byte output
length.

The digest is a deterministic function of the input alone: the same message
always produces the same 64 bytes, with no keying, salting, or randomness. The
function is **total** — every input, including the empty message, yields a digest
and it never raises an error.

The hash is a portable software core computed over the `bits` package, so its
output is **byte-identical on every target** (macOS/Linux, aarch64/x86-64) and
uses no platform crypto library. A digest is raw binary, not text; stringify it
with `encoding::hexEncode` or `encoding::base64Encode` to display or store it.

`sha512` is a general-purpose digest and message-integrity primitive. It is
**not** a password hash on its own; derive password material with
`crypto::pbkdf2Sha512`, and authenticate messages under a shared key with
`crypto::hmacSha512`. The `List OF Byte` overload hashes the raw bytes as given;
the `String` overload hashes the string's UTF-8 encoding."#;
const EX: &str = r#"Hash a byte list and print it as hex:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = strings::toBytes("hello")
  LET digest AS List OF Byte = crypto::sha512(raw)
  io::print(encoding::hexEncode(digest))
END SUB
```

Hash a string (its UTF-8 bytes):

```
IMPORT crypto
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::hexEncode(crypto::sha512("hello")))
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sha512",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF Byte or String"),
        implementations: vec![
            Implementation {
                params: vec![Parameter {
                    name: "data",
                    desc: "The bytes to hash. Any length is accepted, including the empty list.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                }],
                return_type: bytes(),
                errors: vec![],
                body: Body::Rewrite("__crypto_sha512_bytes"),
            },
            Implementation {
                params: vec![Parameter {
                    name: "data",
                    desc: "A string whose UTF-8 bytes are hashed.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                }],
                return_type: bytes(),
                errors: vec![],
                body: Body::Rewrite("__crypto_sha512_text"),
            },
        ],
    });
}
