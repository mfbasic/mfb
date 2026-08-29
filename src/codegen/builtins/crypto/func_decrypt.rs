//! `crypto::decrypt(cipher, recipientPrivateKey, box[, aad])` — asymmetric
//! public-key decryption (the inverse of `crypto::encrypt`).
//!
//! Selected by a [`crypto::AsymmetricCipher`] enum (`Ed25519_*` on DHKEM(X25519,
//! HKDF-SHA256), `Ed448_*` on DHKEM(X448, HKDF-SHA512)), this member rewrites onto
//! the pure-MFB `__crypto_decrypt` core (registered by [`super::helper_decrypt`]) —
//! no platform library, no `AbiFunction`, so (like `hmac`/`hkdf`/`convert`) it is
//! NOT in any backend's `runtime_calls`. It converts the recipient's Ed25519/Ed448
//! seed to its X25519/X448 scalar internally, splits the suite's `Nenc`-byte `enc`
//! off the front of the box, runs RFC 9180 `Decap` and the base key schedule, opens
//! the AEAD at sequence 0, and fails closed with `ErrAuthenticationFailed` on any
//! tamper / wrong key / wrong `aad`.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Decrypt an RFC 9180 HPKE box (`enc ‖ ct`) with the recipient's private key, selected by a `crypto::AsymmetricCipher`."#;
const DESC: &str = r#"`crypto::decrypt(cipher, recipientPrivateKey, box)` recovers the plaintext of a box
produced by `crypto::encrypt(cipher, …)` — or by any RFC 9180 HPKE implementation's
single-shot base-mode `Seal` with the same ciphersuite, empty `info`, and this
`aad` — returning it as a `List OF Byte`. Only the holder of the recipient's
private key can decrypt.

**Inputs.** `cipher` is the same `crypto::AsymmetricCipher` used to encrypt: the
`Ed25519_*` suites are DHKEM(X25519, HKDF-SHA256) + HKDF-SHA256 (`Nenc` = 32), the
`Ed448_*` suites DHKEM(X448, HKDF-SHA512) + HKDF-SHA512 (`Nenc` = 56), each with
AES-256-GCM (`*_AES256GCM`) or ChaCha20Poly1305 (`*_CHACHA20POLY1305`).
`recipientPrivateKey` is the recipient's **signing** private key for the suite's
curve — the 32-byte Ed25519 seed from `crypto::generate(Certificate.Ed25519)`, or
the 57-byte Ed448 seed from `crypto::generate(Certificate.Ed448)` — converted to
its X25519/X448 scalar internally; another length raises `ErrInvalidArgument`.
`box` is RFC 9180's `enc (Nenc bytes) ‖ ct` where `ct` is the AEAD
`ciphertext ‖ tag (16 bytes)`.

**How it works (RFC 9180 §6.1 `Open`).** The suite's `Nenc`-byte `enc` is read from
the front of the box; `Decap` computes `dh = DH(skR, enc)` (X25519 or X448) and the
KEM shared secret `LabeledExpand(LabeledExtract("", "eae_prk", dh),
"shared_secret", enc ‖ pkR, Nsecret)` (`Nsecret` = 32 or 64); the base-mode
`KeySchedule` with empty `info` re-derives the same AEAD key and `base_nonce`; and
the AEAD opens `ct` under the sequence-0 nonce, verifying the tag in **constant
time**.

**Fails closed.** A tampered or truncated box, a wrong recipient key, a different
suite than was used to encrypt (including the same AEAD on the other curve), a
different `aad` than was encrypted, or a box in the pre-RFC `mfb-box-v1` format
this construction replaced, raises `ErrAuthenticationFailed` and returns no
plaintext. A box shorter than the `enc ‖ tag` overhead (48 bytes for an
`Ed25519_*` suite, 72 for an `Ed448_*` suite), or an `enc` that is a low-order
point (the DH output is all zeros), is malformed and raises `ErrInvalidArgument`.
The optional `aad` must match the `aad` supplied to `crypto::encrypt`; it defaults
to the empty list.

**A successful decrypt does not authenticate the sender.** A valid tag proves the
box was not modified, not *who* created it — a sealed box is anonymous, and anyone
holding your public key can send one. When you need to know who sent a message,
have the sender sign it with `crypto::sign` and check the signature with
`crypto::verify` in addition to decrypting.

**Implementation.** X25519 and X448 (RFC 7748), the Ed25519→X25519 and
Ed448→X448 conversions (RFC 8032 / RFC 7748), the HPKE labeled HKDF-SHA256 /
HKDF-SHA512 (RFC 9180 / RFC 5869), and the AEAD are all pure MFBASIC software
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
                    ty: ParameterType::named("AsymmetricCipher"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "recipientPrivateKey",
                    desc: "The recipient's signing private key (seed) for the suite's curve (32-byte Ed25519 for `Ed25519_*`, 57-byte Ed448 for `Ed448_*`); converted to X25519/X448 internally.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "box",
                    desc: "The RFC 9180 `enc ‖ ct` value returned by `crypto::encrypt` (or any conformant HPKE seal with the same suite).",
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
