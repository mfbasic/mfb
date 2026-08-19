//! `crypto::encrypt(cipher, recipientPublicKey, data[, aad])` — asymmetric
//! public-key encryption (an X25519 sealed box that takes Ed25519 keys).
//!
//! Selected by a [`crypto::AsymmetricCipher`] enum
//! (`Ed25519_AES256GCM`/`Ed25519_CHACHA20POLY1305`), this member rewrites onto the
//! pure-MFB `__crypto_encrypt` core (registered by [`super::helper_encrypt`]) — no
//! platform library, no `AbiFunction`, so (like `hmac`/`hkdf`/`convert`) it is NOT in
//! any backend's `runtime_calls`. It converts the recipient's Ed25519 public key to
//! X25519 internally, generates an ephemeral X25519 key pair, and returns the
//! self-contained box `ephPub ‖ ciphertext ‖ tag`.
//!
//! Two overloads mirror `seal`'s `data` typing: the `List OF Byte` form rewrites to
//! `__crypto_encrypt`, the `String` form to the `__crypto_encryptText` UTF-8 shim
//! (both pure MFB, so no `AbiFunction`-symbol collision). `aad` is a trailing
//! optional parameter filling to the empty byte list.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Encrypt a message to a recipient's public key with an X25519 sealed box, selected by a `crypto::AsymmetricCipher`."#;
const DESC: &str = r#"`crypto::encrypt(cipher, recipientPublicKey, data)` encrypts `data` so that only
the holder of the matching private key can read it, and returns a self-contained
`List OF Byte` box. This is public-key (asymmetric) encryption: the sender needs
only the recipient's public key.

**Cipher suite.** `cipher` is a `crypto::AsymmetricCipher` selecting the inner
AEAD: `Ed25519_AES256GCM` (AES-256-GCM) or `Ed25519_CHACHA20POLY1305`
(ChaCha20-Poly1305). `recipientPublicKey` is the recipient's 32-byte **Ed25519**
public key (from `crypto::generate(Certificate.Ed25519)`); it is converted to its
X25519 (Curve25519) form internally, so a single Ed25519 identity serves both
signing and encryption.

**Construction (X25519 sealed box).** Per call:

1. a fresh ephemeral X25519 key pair is generated;
2. an X25519 ECDH (RFC 7748) shared secret is computed between the ephemeral
   private key and the recipient's X25519 public key;
3. `HKDF-SHA256` (RFC 5869) derives 44 bytes from that secret — salt =
   `ephemeralPublicKey ‖ recipientX25519PublicKey`, info = `"mfb-box-v1" ‖
   suiteOrdinal` — split into a 32-byte AEAD key and a 12-byte nonce;
4. `data` is AEAD-sealed under that key and nonce.

The returned box is `ephemeralPublicKey (32 bytes) ‖ ciphertext (= |data|) ‖ tag
(16 bytes)`, decrypted with `crypto::decrypt(cipher, recipientPrivateKey, box)`.

**Non-deterministic.** The random ephemeral key makes the box non-deterministic —
encrypting the same message twice yields different boxes — and gives a measure of
forward secrecy: a captured box's ephemeral secret is discarded after the call, so
later compromise of the recipient's long-term key does not by itself reconstruct
it. The optional `aad` (additional authenticated data) is authenticated but not
encrypted and must be supplied identically to `crypto::decrypt`; it defaults to the
empty list. The `List OF Byte` overload encrypts the raw bytes; the `String`
overload encrypts the string's UTF-8 encoding.

**Not an interoperable wire format.** This is a bespoke MFB construction (note the
`"mfb-box-v1"` domain separation). It is NOT RFC 9180 HPKE and NOT the libsodium
sealed-box wire format — do not expect interop with external libraries; both ends
must use MFB's `crypto::encrypt` / `crypto::decrypt`.

**Implementation.** X25519 (RFC 7748), the Ed25519→X25519 public-key conversion
(RFC 8032 / RFC 7748), HKDF-SHA256 (RFC 5869), and the inner AEAD are all pure
MFBASIC software cores computed over the `bits` package — no platform cryptographic
library — so a box is byte-for-byte wire-compatible across every target (macOS,
Linux, Windows; aarch64, x86-64)."#;
const EX: &str = r#"```
IMPORT crypto

SUB main()
  LET recip AS crypto::KeyPair = crypto::generate(Certificate.Ed25519)
  LET box AS List OF Byte = crypto::encrypt(AsymmetricCipher.Ed25519_AES256GCM, recip.publicKey, "attack at dawn")
  LET clear AS List OF Byte = crypto::decrypt(AsymmetricCipher.Ed25519_AES256GCM, recip.privateKey, box)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    let cipher_param = || Parameter {
        name: "cipher",
        desc: "The asymmetric cipher suite to use.",
        aliases: &[],
        ty: ParameterType::Named("AsymmetricCipher"),
        default: DefaultValue::None,
    };
    let recip_param = || Parameter {
        name: "recipientPublicKey",
        desc: "The recipient's 32-byte Ed25519 public key; converted to X25519 internally.",
        aliases: &[],
        ty: bytes(),
        default: DefaultValue::None,
    };
    let aad_param = || {
        Parameter {
        name: "aad",
        desc: "Optional additional authenticated data; must match `crypto::decrypt`. Defaults to empty.",
        aliases: &[],
        ty: bytes(),
        default: DefaultValue::Fill {
            type_name: bytes(),
            expr: "",
        },
    }
    };
    pkg.add_function(RegistryFunction {
        name: "encrypt",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some(
            "crypto::AsymmetricCipher, List OF Byte, List OF Byte or String[, List OF Byte]",
        ),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    cipher_param(),
                    recip_param(),
                    Parameter {
                        name: "data",
                        desc: "The message bytes to encrypt. Any length is accepted, including the empty list.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    aad_param(),
                ],
                return_type: bytes(),
                errors: vec!["ErrInvalidArgument"],
                body: Body::Rewrite("__crypto_encrypt"),
            },
            Implementation {
                params: vec![
                    cipher_param(),
                    recip_param(),
                    Parameter {
                        name: "data",
                        desc: "A string whose UTF-8 bytes are encrypted.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                    aad_param(),
                ],
                return_type: bytes(),
                errors: vec!["ErrInvalidArgument"],
                body: Body::Rewrite("__crypto_encryptText"),
            },
        ],
    });
}
