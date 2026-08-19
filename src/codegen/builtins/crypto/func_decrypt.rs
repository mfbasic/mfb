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
const DESC: &str = r#"`crypto::decrypt(cipher, recipientPrivateKey, box)` recovers the plaintext of a
box produced by `crypto::encrypt(cipher, …)`, returning it as a `List OF Byte`.
`cipher` is the same `crypto::AsymmetricCipher` used to encrypt; `recipientPrivateKey`
is the recipient's 32-byte Ed25519 private key (the seed from
`crypto::generate(crypto::Certificate.Ed25519)`), converted to X25519 internally.

The box is `ephemeralPublicKey (32 bytes) ‖ ciphertext ‖ tag (16 bytes)`. Decryption
does the ECDH against the embedded ephemeral key, re-derives the AEAD key and nonce
with `HKDF(SHA-256)`, and verifies the tag in **constant time**, **failing closed**:
a tampered box, a wrong recipient key, or a different `aad` than was encrypted raises
`ErrAuthenticationFailed` and returns no plaintext. The optional `aad` must match the
`aad` supplied to `crypto::encrypt`; it defaults to the empty list. The whole
construction is a portable software core over the `bits` package and uses no platform
crypto library."#;
const EX: &str = r#"```
IMPORT crypto

SUB main()
  LET recip AS crypto::KeyPair = crypto::generate(crypto::Certificate.Ed25519)
  LET box AS List OF Byte = crypto::encrypt(crypto::AsymmetricCipher.Ed25519_CHACHA20POLY1305, recip.publicKey, "attack at dawn")
  LET clear AS List OF Byte = crypto::decrypt(crypto::AsymmetricCipher.Ed25519_CHACHA20POLY1305, recip.privateKey, box)
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
