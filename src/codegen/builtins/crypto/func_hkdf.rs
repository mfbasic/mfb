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
const DESC: &str = r#"`crypto::hkdf(type, ikm, salt, info, length)` derives `length` bytes of output
keying material from the input keying material `ikm`, using the HKDF
Extract-then-Expand construction over the SHA-2 hash selected by `type` — a
`crypto::Hash`: `SHA224`, `SHA256`, `SHA384`, or `SHA512`. The result is returned as
a raw `List OF Byte` of exactly `length` bytes. This one call replaces the per-digest
HKDF members behind a single `Hash`-selected surface.

HKDF first *extracts* a fixed-length pseudorandom key from `ikm` and `salt` (one
HMAC), then *expands* it under `info` into the requested output length. `salt` is an
optional, non-secret value that strengthens the extraction; it may be empty, in which
case it is treated as a string of `L` zero bytes, where `L` is the digest length of
`type` (28/32/48/64 for `SHA224`/`SHA256`/`SHA384`/`SHA512`). `info` is optional
context/application binding — a label that domain-separates independent keys derived
from the same `ikm` (for example `"app v1 encryption"` vs `"app v1 signing"`); it may
be empty. The derivation is deterministic in all of its inputs.

`length` must be between 1 and `255 * L` bytes inclusive; a `length` of 0 or greater
than `255 * L` raises `ErrInvalidArgument`. `ikm`, `salt`, and `info` may each be any
length, including empty.

HKDF is a key-derivation function for **already-high-entropy** input (a shared
secret, a Diffie-Hellman result); it is **not** a password hash — stretch a
low-entropy password with `crypto::pbkdf2` instead. Derived key material is raw
binary, not text — stringify it with `encoding::hexEncode` or
`encoding::base64Encode` to display or store it.

**Implementation.** HKDF is specified by RFC 5869 (HMAC-based Extract-then-Expand),
here layered over HMAC of the selected SHA-2 hash. The derivation is computed
in-process by a portable MFBASIC software core over the `bits` package — no platform
cryptographic library is called — so the output is **byte-identical on macOS, Linux,
and Windows** (and across aarch64/x86-64). The core is hash-generic over the `Hash`
enum, so a future `Hash` variant is supported without new code."#;
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
                    desc: "Number of output bytes to derive: 1 to 255*L, where L is the \
                           digest length of `type` (28/32/48/64).",
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
