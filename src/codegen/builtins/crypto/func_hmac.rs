//! `crypto::hmac(type, key, data)` — the unified hash-generic HMAC entry point.
//!
//! Selected by a [`crypto::Hash`] enum (`SHA224`/`SHA256`/`SHA384`/`SHA512`), this
//! member rewrites onto the hash-generic `__crypto_hmac` MFB core (registered by
//! [`super::helper_hmac`]), which computes RFC 2104 HMAC over the `__crypto_shaDigest` /
//! `__crypto_shaBlockSize` dispatch — so every `Hash` variant, present and future, is
//! authenticated by one construction. It is the unified front door for the per-digest
//! `hmacSha256`/`hmacSha512` members (mirroring `hash` over `Hash`).
//!
//! Two overloads mirror the `hash`/`hmacSha*` members. The `List OF Byte` `data` form
//! rewrites to `__crypto_hmac`; the `String` form rewrites to the `__crypto_hmacText`
//! shim (registered by [`super::helper_hmac_text`]) which UTF-8-encodes the string and
//! re-enters the bytes path — a `String` and a `List OF Byte` are not ABI-interchangeable,
//! so the two overloads rewrite to distinct MFB bodies, exactly as `hmacSha256` does.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Compute the HMAC message authentication code (RFC 2104) of a message under a key, selected by a `crypto::Hash`."#;
const DESC: &str = r#"`crypto::hmac(type, key, data)` computes the keyed-hash message authentication
code of `data` under `key`, using the SHA-2 hash selected by `type` (a
`crypto::Hash`: `SHA224`, `SHA256`, `SHA384`, or `SHA512`), as specified by
RFC 2104. It returns the MAC as a `List OF Byte` — 28, 32, 48, or 64 bytes
respectively. It is the unified front door for the per-digest HMAC members
(`hmacSha256`/`hmacSha512`) behind one `Hash`-selected call.

Keys of any length are accepted. Per RFC 2104, a key longer than the hash's block
size (64 bytes for `SHA224`/`SHA256`, 128 bytes for `SHA384`/`SHA512`) is first
hashed down to the digest length, and a shorter key is right-padded with zero
bytes to the block size before the inner and outer passes.

The MAC is a deterministic function of `type`, `key`, and `data` alone: the same
inputs always produce the same bytes, with no salting or randomness. The function
is **total** — every combination of inputs, including empty key and empty message,
yields a MAC and it never raises an error.

The MAC is a portable software core computed over the `bits` package, so its
output is **byte-identical on every target** (macOS/Linux/Windows, aarch64/x86-64)
and uses no platform crypto library. A MAC is raw binary, not text; stringify it
with `encoding::hexEncode` or `encoding::base64Encode` to display or store it. To
compare a received MAC against a computed one, use `crypto::constantTimeEqual` so
the comparison does not leak timing information. The `List OF Byte` overload
authenticates the raw bytes as given; the `String` overload authenticates the
string's UTF-8 encoding."#;
const EX: &str = r#"Authenticate a message under SHA-256 and print the MAC as hex:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET mac AS List OF Byte = crypto::hmac(Hash.SHA256, key, message)
  io::print(encoding::hexEncode(mac))
END SUB
```

Authenticate a string under a different digest:

```
IMPORT crypto
IMPORT encoding
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(64)
  io::print(encoding::hexEncode(crypto::hmac(Hash.SHA512, key, "payload")))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hmac",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("crypto::Hash, List OF Byte, (List OF Byte or String)"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "type",
                        desc: "The hash algorithm underlying the HMAC.",
                        aliases: &[],
                        ty: ParameterType::Named("Hash"),
                        default: DefaultValue::None,
                    },
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
                body: Body::Rewrite("__crypto_hmac"),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "type",
                        desc: "The hash algorithm underlying the HMAC.",
                        aliases: &[],
                        ty: ParameterType::Named("Hash"),
                        default: DefaultValue::None,
                    },
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
                body: Body::Rewrite("__crypto_hmacText"),
            },
        ],
    });
}
