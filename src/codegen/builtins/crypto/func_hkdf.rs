//! `crypto::hkdf(type, ikm, salt, info, length)` — the unified hash-generic HKDF entry.
//!
//! Selected by a [`crypto::Hash`] enum (`SHA224`/`SHA256`/`SHA384`/`SHA512`), this
//! member rewrites onto the hash-generic `__crypto_hkdf` MFB core (registered by
//! [`super::helper_hkdf`]), which computes RFC 5869 Extract+Expand over the hash-generic
//! `__crypto_hmac` — so every `Hash` variant, present and future, derives keys through
//! one construction. It is the unified front door for the per-digest `hkdfSha256`/
//! `hkdfSha512` members.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Derive key material with HKDF (RFC 5869), selected by a `crypto::Hash`."#;
const DESC: &str = r#"`crypto::hkdf(type, ikm, salt, info, length)` runs the HKDF Extract-then-Expand
key-derivation function of RFC 5869 over the SHA-2 hash selected by `type` (a
`crypto::Hash`: `SHA224`, `SHA256`, `SHA384`, or `SHA512`), turning the input
keying material `ikm` into `length` bytes of output keying material, returned as a
`List OF Byte`. It is the unified front door for the per-digest HKDF members
(`hkdfSha256`/`hkdfSha512`) behind one `Hash`-selected call.

`salt` is an optional non-secret value that may be empty (an empty salt is treated
as a string of zero bytes the length of the digest); `info` is optional
context/application binding that may be empty. `length` must be between 1 and
`255 * L` bytes, where `L` is the digest length of `type` (28/32/48/64 for
`SHA224`/`SHA256`/`SHA384`/`SHA512`); a `length` outside that range raises
`ErrInvalidArgument`.

The derivation is a deterministic function of its inputs alone, and is a portable
software core computed over the `bits` package, so its output is **byte-identical
on every target** (macOS/Linux/Windows, aarch64/x86-64) and uses no platform crypto
library. Derived key material is raw binary, not text; stringify it with
`encoding::hexEncode` or `encoding::base64Encode` to display or store it."#;
const EX: &str = r#"Derive a 32-byte key from input keying material:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET ikm AS List OF Byte = strings::toBytes("shared secret")
  LET salt AS List OF Byte = crypto::randomBytes(16)
  LET info AS List OF Byte = strings::toBytes("app v1 encryption")
  LET key AS List OF Byte = crypto::hkdf(Hash.SHA256, ikm, salt, info, 32)
  io::print(encoding::hexEncode(key))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hkdf",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("crypto::Hash, List OF Byte, List OF Byte, List OF Byte, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "type",
                    desc: "The hash algorithm underlying the HKDF.",
                    aliases: &[],
                    ty: ParameterType::Named("Hash"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "ikm",
                    desc: "Input keying material.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "salt",
                    desc: "Optional non-secret salt; may be empty.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "info",
                    desc: "Optional context/application info; may be empty.",
                    aliases: &[],
                    ty: bytes(),
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
            body: Body::Rewrite("__crypto_hkdf"),
        }],
    });
}
