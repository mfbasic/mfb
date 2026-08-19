//! `crypto::decrypt(cipher, recipientPrivateKey, box[, aad])` — asymmetric
//! public-key decryption (the inverse of `crypto::encrypt`).
//!
//! Selected by a [`crypto::AsymmetricCipher`] enum
//! (`Ed25519_AES256GCM`/`Ed25519_CHACHA20POLY1305`), this member rewrites onto the
//! pure-MFB `__crypto_decrypt` core (registered by [`super::helper_decrypt`]) — no
//! platform library, no `AbiFunction`, so (like `hmac`/`hkdf`/`convert`) it is NOT in
//! any backend's `runtime_calls`. It converts the recipient's Ed25519 seed to its
//! X25519 scalar internally, recovers the ephemeral public key from the box, does the
//! ECDH, re-derives the AEAD key/nonce, and fails closed with
//! `ErrAuthenticationFailed` on any tamper / wrong key / wrong `aad`.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Decrypt an X25519 sealed box with the recipient's private key, selected by a `crypto::AsymmetricCipher`."#;
const DESC: &str = r#"`crypto::decrypt(cipher, recipientPrivateKey, box)` recovers the plaintext of a box
produced by `crypto::encrypt(cipher, …)`, returning it as a `List OF Byte`. Only the
holder of the recipient's private key can decrypt.

**Inputs.** `cipher` is the same `crypto::AsymmetricCipher` used to encrypt
(`Ed25519_AES256GCM` or `Ed25519_CHACHA20POLY1305`). `recipientPrivateKey` is the
recipient's 32-byte **Ed25519** private key (the seed from
`crypto::generate(Certificate.Ed25519)`), converted to its X25519 scalar
internally. `box` is the self-contained
`ephemeralPublicKey (32 bytes) ‖ ciphertext ‖ tag (16 bytes)` returned by
`crypto::encrypt`.

**How it works.** The 32-byte ephemeral public key is read from the front of the
box, an X25519 ECDH (RFC 7748) is performed against it, `HKDF-SHA256` (RFC 5869)
re-derives the same 32-byte key and 12-byte nonce, and the inner AEAD tag is
verified in **constant time**.

**Fails closed.** A tampered or truncated box, a wrong recipient key, or a different
`aad` than was encrypted raises `ErrAuthenticationFailed` and returns no plaintext.
A box shorter than the 48-byte `ephemeralPublicKey ‖ tag` overhead is malformed and
raises `ErrInvalidArgument`. The optional `aad` must match the `aad` supplied to
`crypto::encrypt`; it defaults to the empty list. Both ends must use MFB's
`crypto::encrypt` / `crypto::decrypt` — the box is a bespoke MFB format, not RFC
9180 HPKE or the libsodium sealed box.

**Implementation.** X25519 (RFC 7748), the Ed25519→X25519 conversion (RFC 8032 /
RFC 7748), HKDF-SHA256 (RFC 5869), and the inner AEAD are all pure MFBASIC software
cores computed over the `bits` package — no platform cryptographic library — so
decryption is byte-identical on every target (macOS, Linux, Windows; aarch64,
x86-64)."#;
const EX: &str = r#"```
IMPORT crypto

SUB main()
  LET recip AS crypto::KeyPair = crypto::generate(Certificate.Ed25519)
  LET box AS List OF Byte = crypto::encrypt(AsymmetricCipher.Ed25519_CHACHA20POLY1305, recip.publicKey, "attack at dawn")
  LET clear AS List OF Byte = crypto::decrypt(AsymmetricCipher.Ed25519_CHACHA20POLY1305, recip.privateKey, box)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "decrypt",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some(
            "crypto::AsymmetricCipher, List OF Byte, List OF Byte[, List OF Byte]",
        ),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "cipher",
                    desc: "The asymmetric cipher suite to use; must match `crypto::encrypt`.",
                    aliases: &[],
                    ty: ParameterType::Named("AsymmetricCipher"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "recipientPrivateKey",
                    desc: "The recipient's 32-byte Ed25519 private key (seed); converted to X25519 internally.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "box",
                    desc: "The self-contained box returned by `crypto::encrypt`.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "aad",
                    desc: "Optional additional authenticated data; must match the `crypto::encrypt` `aad`. Defaults to empty.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::Fill {
                        type_name: bytes(),
                        expr: "",
                    },
                },
            ],
            return_type: bytes(),
            errors: vec!["ErrAuthenticationFailed", "ErrInvalidArgument"],
            body: Body::Rewrite("__crypto_decrypt"),
        }],
    });
}
