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

const INTRO: &str = r#"Derive a key from a password with PBKDF2, selected by a `crypto::Hash`."#;
const DESC: &str = r#"`crypto::pbkdf2(type, password, salt, iterations, length)` derives `length` bytes of
key material from a `password` and `salt` using PBKDF2-HMAC over the SHA-2 hash
selected by `type` — a `crypto::Hash`: `SHA224`, `SHA256`, `SHA384`, or `SHA512`. The
result is returned as a raw `List OF Byte` of exactly `length` bytes. This one call
replaces the per-digest PBKDF2 members behind a single `Hash`-selected surface.

PBKDF2 applies the underlying HMAC `iterations` times per output block, chaining the
salted password through repeated hashing so that each derived byte costs about
`iterations` HMAC evaluations. This iteration count is the *work factor*: it
deliberately slows the derivation to make brute-force guessing of the password
expensive. Use a unique, random `salt` per password (16 bytes or more) to defeat
precomputation, and set `iterations` as high as your latency budget allows — for
PBKDF2-HMAC-SHA256 current guidance (OWASP, 2023) is on the order of **600,000**
iterations, scaled down for the larger SHA-384/512 blocks and up for offline use.
The derivation is deterministic in all of its inputs.

**PBKDF2 is not memory-hard.** It is CPU-only and cheap to parallelize on GPUs and
ASICs, so an attacker's per-guess cost is far lower than yours. For *storing*
passwords, prefer a memory-hard function (Argon2id, scrypt, or bcrypt) where one is
available; reach for PBKDF2 mainly to derive a key from a passphrase or for
compatibility with an existing PBKDF2 deployment.

`iterations` and `length` must each be at least 1; a value below 1 for either raises
`ErrInvalidArgument`. `password` and `salt` may be any length, including empty. There
is no upper bound beyond available time and memory — a large `iterations` or `length`
simply takes proportionally longer.

PBKDF2 is a password-stretching KDF; to derive keys from already-high-entropy input
use `crypto::hkdf` instead, and to authenticate a message use `crypto::hmac`. Derived
key material is raw binary, not text — stringify it with `encoding::hexEncode` or
`encoding::base64Encode`, and compare a stored derivation against a recomputed one
with `crypto::constantTimeEqual`.

**Implementation.** PBKDF2 is specified by RFC 8018 (PKCS#5 v2.1), here layered over
HMAC of the selected SHA-2 hash. The derivation is computed in-process by a portable
MFBASIC software core over the `bits` package — no platform cryptographic library is
called — so the output is **byte-identical on macOS, Linux, and Windows** (and across
aarch64/x86-64). The core is hash-generic over the `Hash` enum, so a future `Hash`
variant is supported without new code."#;
const EX: &str = r#"Derive a 32-byte key from a password:

```
IMPORT crypto
IMPORT strings
IMPORT io

SUB main()
  LET password AS List OF Byte = strings::toBytes("correct horse")
  LET salt AS List OF Byte = crypto::randomBytes(16)
  LET key AS List OF Byte = crypto::pbkdf2(Hash.SHA256, password, salt, 600000, 32)
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
                    desc: "PBKDF2 iteration count (the work factor); must be at least 1.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "length",
                    desc: "Number of output bytes to derive; must be at least 1.",
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
