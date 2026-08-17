//! `crypto::aes256GcmOpen` — descriptor entry + authored docs.
//!
//! An AEAD source member: the verify-and-decrypt half of AES-256-GCM. Fails closed on
//! any tag mismatch via `crypto::constantTimeEqual`. Its
//! `Body::Rewrite("__crypto_aes256GcmOpen")` repoints the citation at the `package.mfb`
//! helper; the optional `aad` parameter fills to the empty byte list. Docs migrated
//! from `src/docs/man/builtins/crypto/aes256GcmOpen.md`.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, RegistryFunction};

const INTRO: &str = r#"Verify and decrypt an AES-256-GCM sealed message, failing closed on any tag mismatch (NIST SP 800-38D)."#;
const DESC: &str = r#"`crypto::aes256GcmOpen` verifies and decrypts a ciphertext produced by
`crypto::aes256GcmSeal`, using AES-256 in Galois/Counter Mode (GCM) as specified
by NIST SP 800-38D. It recomputes the authentication tag over the `ciphertext`
and the additional authenticated data, compares that tag against the supplied
`tag` in constant time, and returns the recovered plaintext only if they match.

The function fails closed. On any tag mismatch it raises
`ErrAuthenticationFailed` and returns no plaintext at all — not a partial or
unverified decryption. The tag is compared with `crypto::constantTimeEqual`, so
the check is content-independent in time and does not leak how much of the tag
matched. A mismatch means the `ciphertext`, `tag`, `nonce`, or `aad` was
altered, truncated, or does not belong to this key; the message must be
rejected.

`key` must be exactly 32 bytes (a 256-bit key) and `nonce` must be exactly 12
bytes (the standard 96-bit GCM nonce); any other length raises
`ErrInvalidArgument`. To open successfully, `key`, `nonce`, `ciphertext`, `tag`,
and `aad` must all be identical to those from the sealing call: the `aad` is
authenticated but not carried in the ciphertext, so the same `aad` must be
supplied here. `aad` defaults to the empty list when omitted. An empty
`ciphertext` is valid and recovers an empty plaintext when the tag over the
`aad` alone verifies.

The cipher is a portable software core computed over the `bits` package, so its
behavior is **byte-identical on every target** (macOS/Linux, aarch64/x86-64) and
uses no platform crypto library."#;
const EX: &str = r#"Round-trip: seal then open:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET plaintext AS List OF Byte = strings::toBytes("hello")
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
  LET box AS crypto::Sealed = crypto::aes256GcmSeal(key, nonce, plaintext)
  LET clear AS List OF Byte = crypto::aes256GcmOpen(key, nonce, box.ciphertext, box.tag)
END SUB
```

A tampered ciphertext, tag, or aad raises `ErrAuthenticationFailed` and returns
nothing:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
  LET plaintext AS List OF Byte = strings::toBytes("hello")
  LET box AS crypto::Sealed = crypto::aes256GcmSeal(key, nonce, plaintext)
  ' If box.tag has been altered in transit, this call fails closed:
  LET clear AS List OF Byte = crypto::aes256GcmOpen(key, nonce, box.ciphertext, box.tag)
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "aes256GcmOpen",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "key",
                    desc: "The 32-byte AES-256 key.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "nonce",
                    desc: "The 12-byte nonce used to seal.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "ciphertext",
                    desc: "The ciphertext bytes to decrypt.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "tag",
                    desc: "The 16-byte authentication tag.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "aad",
                    desc: "Optional additional authenticated data; defaults to empty. Must match the value used to seal.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::Fill {
                        type_name: bytes(),
                        expr: "",
                    },
                },
            ],
            return_type: bytes(),
            errors: vec!["ErrInvalidArgument", "ErrAuthenticationFailed"],
            body: Body::Rewrite("__crypto_aes256GcmOpen"),
        }],
    });
}
