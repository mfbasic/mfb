//! `crypto::hkdfSha512` — descriptor entry + authored docs.
//!
//! A single-overload SOURCE member: the HKDF (RFC 5869) key-derivation function
//! over HMAC-SHA-512, a portable software core whose `Body::Rewrite` repoints at
//! `__crypto_hkdfSha512` in `package.mfb`. Docs migrated from
//! `src/docs/man/builtins/crypto/hkdfSha512.md`.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Derive key material with HKDF (RFC 5869) instantiated over HMAC-SHA-512."#;
const DESC: &str = r#"`crypto::hkdfSha512` is the HKDF key-derivation function of RFC 5869 instantiated
over HMAC-SHA-512. It turns input keying material of arbitrary quality into one
or more cryptographically strong keys of a chosen length. HKDF runs in two
phases: an **extract** step folds `ikm` and `salt` into a fixed-length
pseudorandom key (the 64-byte HMAC-SHA-512 output), and an **expand** step
stretches that key into `length` output bytes bound to the `info` context.

`salt` is optional in the RFC sense: passing an empty list selects HKDF's default
all-zero salt of one hash block (64 bytes). `info` may also be empty; when
non-empty it domain-separates derived keys, so the same `ikm` can safely produce
independent keys for different purposes.

`length` must be at least 1 and at most `255 * 64 = 16320` bytes — the ceiling
imposed by HKDF-Expand's single-byte block counter over a 64-byte hash. A
`length` of 0 or below, or above 16320, raises `ErrInvalidArgument`.

The function is deterministic and total within its `length` bound: the same four
arguments always yield the same bytes. Because it is a portable software core
computed over the `bits` package, its output is **byte-identical on every
target** (macOS/Linux, aarch64/x86-64) and uses no platform crypto library.

HKDF is designed for high-entropy `ikm` (for example a Diffie-Hellman shared
secret). To derive keys from a low-entropy password, use `crypto::pbkdf2Sha512`
instead. Derived key material is raw binary; to display or store it, stringify it
with `encoding::hexEncode` or `encoding::base64Encode`."#;
const EX: &str = r#"Derive a 64-byte key from a shared secret:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET secret AS List OF Byte = crypto::randomBytes(32)
  LET salt AS List OF Byte = crypto::randomBytes(16)
  LET info AS List OF Byte = strings::toBytes("app v1")
  LET key AS List OF Byte = crypto::hkdfSha512(secret, salt, info, 64)
END SUB
```

An empty salt selects the RFC default all-zero salt:

```
IMPORT crypto

SUB main()
  LET secret AS List OF Byte = crypto::randomBytes(32)
  LET empty AS List OF Byte = []
  LET key AS List OF Byte = crypto::hkdfSha512(secret, empty, empty, 128)
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hkdfSha512",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![
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
            body: Body::Rewrite("__crypto_hkdfSha512"),
        }],
    });
}
