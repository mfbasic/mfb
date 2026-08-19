//! `crypto::pbkdf2(type, password, salt, iterations, length)` — unified hash-generic
//! PBKDF2 entry point.
//!
//! Selected by a [`crypto::Hash`] enum (`SHA224`/`SHA256`/`SHA384`/`SHA512`), this
//! member rewrites onto the hash-generic `__crypto_pbkdf2` MFB core (registered by
//! [`super::helper_pbkdf2`]), which runs RFC 2898 PBKDF2 over the hash-generic
//! `__crypto_hmac` — so every `Hash` variant, present and future, derives password keys
//! through one construction. It is the unified front door for the per-digest
//! `pbkdf2Sha256`/`pbkdf2Sha512` members.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str =
    r#"Derive a key from a password with PBKDF2 (RFC 2898), selected by a `crypto::Hash`."#;
const DESC: &str = r#"`crypto::pbkdf2(type, password, salt, iterations, length)` derives `length` bytes
of key material from `password` and `salt` using PBKDF2-HMAC (RFC 2898) with the
SHA-2 hash selected by `type` (a `crypto::Hash`: `SHA224`, `SHA256`, `SHA384`, or
`SHA512`), applying `iterations` rounds of the underlying HMAC. The result is
returned as a `List OF Byte`. It is the unified front door for the per-digest
PBKDF2 members (`pbkdf2Sha256`/`pbkdf2Sha512`) behind one `Hash`-selected call.

`iterations` and `length` must each be at least 1; otherwise `ErrInvalidArgument`
is raised. Choose the iteration count as high as your latency budget allows — it
is the work factor that slows brute-force guessing of the password.

The derivation is a deterministic function of its inputs alone, and is a portable
software core computed over the `bits` package, so its output is **byte-identical
on every target** (macOS/Linux/Windows, aarch64/x86-64) and uses no platform crypto
library. Derived key material is raw binary, not text; stringify it with
`encoding::hexEncode` or `encoding::base64Encode` to display or store it."#;
const EX: &str = r#"Derive a 32-byte key from a password:

```
IMPORT crypto
IMPORT strings
IMPORT io

SUB main()
  LET password AS List OF Byte = strings::toBytes("correct horse")
  LET salt AS List OF Byte = crypto::randomBytes(16)
  LET key AS List OF Byte = crypto::pbkdf2(Hash.SHA256, password, salt, 100000, 32)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pbkdf2",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("crypto::Hash, List OF Byte, List OF Byte, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "type",
                    desc: "The hash algorithm underlying the PBKDF2.",
                    aliases: &[],
                    ty: ParameterType::Named("Hash"),
                    default: DefaultValue::None,
                },
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
            body: Body::Rewrite("__crypto_pbkdf2"),
        }],
    });
}
