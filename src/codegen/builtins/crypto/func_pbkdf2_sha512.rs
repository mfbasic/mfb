//! `crypto::pbkdf2Sha512` — descriptor entry + authored docs.
//!
//! A two-overload SOURCE member: the `List OF Byte` and `String` password forms
//! are distinct `Implementation`s whose parameter type makes `select` pick the
//! `_bytes`/`_text` `Body::Rewrite` body in `package.mfb`. Docs migrated from
//! `src/docs/man/builtins/crypto/pbkdf2Sha512.md`.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str = r#"Derive a key from a password with PBKDF2 (RFC 8018) over HMAC-SHA-512."#;
const DESC: &str = r#"`crypto::pbkdf2Sha512` is the PBKDF2 password-based key-derivation function of
RFC 8018 instantiated over HMAC-SHA-512. It stretches a low-entropy `password`
into `length` bytes of derived key material by iterating the HMAC core
`iterations` times per output block, deliberately making brute-force guessing of
the password proportionally more expensive.

The output is produced one 64-byte HMAC-SHA-512 block at a time until at least
`length` bytes have accumulated, then truncated to exactly `length` bytes. Each
block folds `salt` and the block index through `iterations` rounds of HMAC,
XOR-accumulating every round into the block.

`salt` should be unique per password and need not be secret; a random salt from
`crypto::randomBytes` (16 bytes or more) is recommended, stored alongside the
derived key. `iterations` sets the work factor and directly trades security for
latency; choose the largest value your latency budget tolerates. This cost is
what distinguishes PBKDF2 from HKDF: use PBKDF2 for passwords, and
`crypto::hkdfSha512` for already-high-entropy keying material.

`password` is overloaded: a `String` argument is UTF-8-encoded internally, so the
`String` and `List OF Byte` forms agree for ASCII and UTF-8 text. The function is
deterministic and total within its argument bounds — the same inputs always yield
the same bytes. Because it is a portable software core computed over the `bits`
package, its output is **byte-identical on every target** (macOS/Linux,
aarch64/x86-64) and uses no platform crypto library. `iterations` and `length`
must each be at least 1, or `ErrInvalidArgument` is raised.

Derived key material is raw binary; to display or store it, stringify it with
`encoding::hexEncode` or `encoding::base64Encode`."#;
const EX: &str = r#"Derive a 64-byte key from a password string:

```
IMPORT crypto

SUB main()
  LET salt AS List OF Byte = crypto::randomBytes(16)
  LET key AS List OF Byte = crypto::pbkdf2Sha512("correct horse", salt, 100000, 64)
END SUB
```

The byte-list form is equivalent for UTF-8 input:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET passwordBytes AS List OF Byte = strings::toBytes("correct horse")
  LET salt AS List OF Byte = crypto::randomBytes(16)
  LET key AS List OF Byte = crypto::pbkdf2Sha512(passwordBytes, salt, 100000, 64)
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pbkdf2Sha512",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("(List OF Byte or String), List OF Byte, Integer, Integer"),
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "password",
                        desc: "The password bytes.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "salt",
                        desc: "The salt bytes.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "iterations",
                        desc: "PBKDF2 iteration count.",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "length",
                        desc: "Number of output bytes to derive.",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                ],
                return_type: bytes(),
                errors: vec!["ErrInvalidArgument"],
                body: Body::Rewrite("__crypto_pbkdf2Sha512_bytes"),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "password",
                        desc: "A password string whose UTF-8 bytes are used.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "salt",
                        desc: "The salt bytes.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "iterations",
                        desc: "PBKDF2 iteration count.",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "length",
                        desc: "Number of output bytes to derive.",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                ],
                return_type: bytes(),
                errors: vec!["ErrInvalidArgument"],
                body: Body::Rewrite("__crypto_pbkdf2Sha512_text"),
            },
        ],
    });
}
