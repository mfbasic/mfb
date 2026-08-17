//! `crypto::chacha20Poly1305Open` — descriptor entry + authored docs.
//!
//! An AEAD source member: the verify-and-decrypt half of ChaCha20-Poly1305 (RFC 8439).
//! Fails closed on any tag mismatch via `crypto::constantTimeEqual`. Its
//! `Body::Rewrite("__crypto_chacha20Poly1305Open")` repoints the citation at the
//! `package.mfb` helper; the optional `aad` parameter fills to the empty byte list.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, RegistryFunction};

const INTRO: &str = r#"Verify and decrypt a ChaCha20-Poly1305 sealed message, failing closed on any tag mismatch (RFC 8439)."#;
const DESC: &str = r#"`crypto::chacha20Poly1305Open` verifies and decrypts a ciphertext produced by
`crypto::chacha20Poly1305Seal`, using the ChaCha20-Poly1305 AEAD construction as
specified by RFC 8439. It recomputes the Poly1305 authentication tag over the
`ciphertext` and the additional authenticated data, compares that tag against the
supplied `tag` in constant time, and returns the recovered plaintext only if they
match.

The function fails closed. On any tag mismatch it raises
`ErrAuthenticationFailed` and returns no plaintext at all — not a partial or
unverified decryption. The tag is compared with `crypto::constantTimeEqual`, so
the check is content-independent in time and does not leak how much of the tag
matched. A mismatch means the `ciphertext`, `tag`, `nonce`, or `aad` was
altered, truncated, or does not belong to this key; the message must be
rejected.

`key` must be exactly 32 bytes (a 256-bit key) and `nonce` must be exactly 12
bytes (the 96-bit RFC 8439 nonce); any other length raises `ErrInvalidArgument`.
To open successfully, `key`, `nonce`, `ciphertext`, `tag`, and `aad` must all be
identical to those from the sealing call: the `aad` is authenticated but not
carried in the ciphertext, so the same `aad` must be supplied here. `aad`
defaults to the empty list when omitted. An empty `ciphertext` is valid and
recovers an empty plaintext when the tag over the `aad` alone verifies.

The cipher is a portable software core computed over the `bits` package, so its
behavior is **byte-identical on every target** (macOS/Linux, aarch64/x86-64) and
uses no platform crypto library. ChaCha20-Poly1305 is a strong choice on targets
without AES hardware acceleration; AES-256-GCM (`crypto::aes256GcmOpen`) is the
interchangeable alternative."#;
const EX: &str = r#"Round-trip: seal then open:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET plaintext AS List OF Byte = strings::toBytes("hello")
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
  LET box AS crypto::Sealed = crypto::chacha20Poly1305Seal(key, nonce, plaintext)
  LET clear AS List OF Byte = crypto::chacha20Poly1305Open(key, nonce, box.ciphertext, box.tag)
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
  LET box AS crypto::Sealed = crypto::chacha20Poly1305Seal(key, nonce, plaintext)
  ' If box.tag has been altered in transit, this call fails closed:
  LET clear AS List OF Byte = crypto::chacha20Poly1305Open(key, nonce, box.ciphertext, box.tag)
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "chacha20Poly1305Open",
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
            body: Body::Rewrite("__crypto_chacha20Poly1305Open"),
        }],
    });
}
