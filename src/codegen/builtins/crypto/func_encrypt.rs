//! `crypto::encrypt(cipher, recipientPublicKey, data[, aad])` — asymmetric
//! public-key encryption: RFC 9180 HPKE single-shot base mode over Ed25519 or
//! Ed448 keys.
//!
//! Selected by a [`crypto::AsymmetricCipher`] enum (`Ed25519_AES256GCM` /
//! `Ed25519_CHACHA20POLY1305` on DHKEM(X25519, HKDF-SHA256), `Ed448_AES256GCM` /
//! `Ed448_CHACHA20POLY1305` on DHKEM(X448, HKDF-SHA512)), this member rewrites onto
//! the pure-MFB `__crypto_encrypt` core (registered by [`super::helper_encrypt`]) —
//! no platform library, no `AbiFunction`, so (like `hmac`/`hkdf`/`convert`) it is
//! NOT in any backend's `runtime_calls`. It converts the recipient's signing public
//! key to the suite's Montgomery curve internally, generates an ephemeral key pair
//! there, runs the RFC 9180 `Encap` + base key schedule (`__crypto_hpkeSealWith`),
//! and returns `enc ‖ ct`.
//!
//! Two overloads mirror `seal`'s `data` typing: the `List OF Byte` form rewrites to
//! `__crypto_encrypt`, the `String` form to the `__crypto_encryptText` UTF-8 shim
//! (both pure MFB, so no `AbiFunction`-symbol collision). `aad` is a trailing
//! optional parameter filling to the empty byte list. The construction is proven
//! against RFC 9180's Appendix A vectors and both-ways against an independent
//! implementation in `tests/rt_crypto_hpke_interop.rs`.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Encrypt a message to a recipient's public key with RFC 9180 HPKE (base mode), selected by a `crypto::AsymmetricCipher`."#;
const DESC: &str = r#"`crypto::encrypt(cipher, recipientPublicKey, data)` encrypts `data` so that only
the holder of the matching private key can read it, and returns a self-contained
`List OF Byte`. This is public-key (asymmetric) encryption: the sender needs only
the recipient's public key. The construction is **RFC 9180 HPKE**, single-shot
**base mode** (`mode_base`, no PSK, no sender authentication), and the returned
value is exactly RFC 9180's `enc ‖ ct` — it interoperates with any conformant HPKE
implementation using the same ciphersuite.

**Cipher suite.** `cipher` is a `crypto::AsymmetricCipher` selecting the RFC 9180
ciphersuite. The `Ed25519_*` suites use `DHKEM(X25519, HKDF-SHA256)` (KEM id
`0x0020`, `Nenc` = 32) with `HKDF-SHA256` (KDF id `0x0001`); the `Ed448_*` suites
use `DHKEM(X448, HKDF-SHA512)` (KEM id `0x0021`, `Nenc` = 56) with `HKDF-SHA512`
(KDF id `0x0003`). The `*_AES256GCM` suites seal with `AES-256-GCM` (AEAD id
`0x0002`) and the `*_CHACHA20POLY1305` suites with `ChaCha20Poly1305` (AEAD id
`0x0003`). `recipientPublicKey` is the recipient's **signing** public key for the
suite's curve — the 32-byte Ed25519 key from `crypto::generate(crypto::Certificate.Ed25519)`
for an `Ed25519_*` suite, the 57-byte Ed448 key from
`crypto::generate(crypto::Certificate.Ed448)` for an `Ed448_*` suite. It is converted to
the KEM curve internally (the `crypto::convert` `Ed25519ToX25519` / `Ed448ToX448`
map), so a single signing identity serves both signing and encryption. A key of
any other length for the selected suite raises `ErrInvalidArgument`.

**Construction (RFC 9180 §6.1 `Seal`).** Per call, with `DH` = X25519 or X448 and
`Nsecret` = 32 or 64 by suite:

1. a fresh ephemeral KEM key pair `(skE, pkE)` is generated — `enc = pkE`;
2. `Encap`: `dh = DH(skE, pkR)`, and the KEM shared secret is
   `LabeledExpand(LabeledExtract("", "eae_prk", dh), "shared_secret", enc ‖ pkR,
   Nsecret)` under the DHKEM suite id (`"KEM" ‖ kem_id`);
3. the base-mode `KeySchedule` with an **empty `info`** derives the AEAD key
   (`Nk` = 32) and `base_nonce` (`Nn` = 12) under the HPKE suite id (`"HPKE" ‖
   kem_id ‖ kdf_id ‖ aead_id`);
4. `data` is AEAD-sealed under that key with the sequence-0 nonce (`base_nonce`)
   and the caller's `aad`.

The returned value is `enc (Nenc bytes) ‖ ct`, where `ct` is the AEAD output —
`ciphertext (= |data|) ‖ tag (16 bytes)` — so the fixed overhead is 48 bytes for an
`Ed25519_*` suite and 72 bytes for an `Ed448_*` suite. It is decrypted with
`crypto::decrypt(cipher, recipientPrivateKey, box)`, or by any RFC 9180
implementation's `Open` with the same suite, empty `info`, and this `aad`. The
all-zero DH output (a low-order recipient key) fails closed with
`ErrInvalidArgument`.

**Non-deterministic.** The random ephemeral key makes the box non-deterministic —
encrypting the same message twice yields different boxes. The optional `aad`
(additional authenticated data) is authenticated but not encrypted and must be
supplied identically to `crypto::decrypt`; it defaults to the empty list. The
`List OF Byte` overload encrypts the raw bytes; the `String` overload encrypts the
string's UTF-8 encoding.

**Security properties — read before relying on it.** The box provides
*confidentiality* and *integrity*, but deliberately **not** two properties
developers often assume:

- **No forward secrecy.** The shared secret is `ECDH(ephemeral, recipient)`, and the
  ephemeral public key travels inside the box, so the recipient's long-term private
  key decrypts *every* box ever sent to it. An attacker who records boxes and later
  obtains that private key can read all of them. The fresh ephemeral key only means
  the *sender* retains no long-term secret; it does not protect past messages once
  the recipient's key leaks. True forward secrecy needs an interactive protocol with
  fresh ephemeral keys on **both** sides.
- **No sender authentication.** Anyone holding the recipient's public key can produce
  a valid box; the AEAD tag proves the box was not modified, not *who* created it. A
  raw sealed box is anonymous — if the recipient must know the sender, also sign the
  message with `crypto::sign`.

Because one signing identity here serves both signing and encryption, see the
key-reuse note on `crypto::convert` before sharing a single key pair across both.

**Interoperable wire format.** The value is RFC 9180 `enc ‖ ct` for the suite
above — the same bytes a conformant HPKE library produces (verified both ways
against an independent implementation and against the RFC's own test vectors).
Values produced by the pre-RFC `mfb-box-v1` construction this replaced are no
longer accepted: `crypto::decrypt` rejects them with `ErrAuthenticationFailed`.
Note it is not the libsodium `crypto_box_seal` format.

**Implementation.** X25519 and X448 (RFC 7748), the Ed25519→X25519 and
Ed448→X448 public-key conversions (RFC 8032 / RFC 7748), the HPKE labeled
HKDF-SHA256 / HKDF-SHA512 (RFC 9180 / RFC 5869), and the AEAD are all pure MFBASIC
software cores computed over the `bits` package — no platform cryptographic
library — so a box is byte-for-byte wire-compatible across every target (macOS,
Linux, Windows; aarch64, x86-64)."#;
const EX: &str = r#"```
IMPORT crypto

SUB main()
  LET recip AS crypto::KeyPair = crypto::generate(crypto::Certificate.Ed25519)
  LET box AS List OF Byte = crypto::encrypt(crypto::AsymmetricCipher.Ed25519_AES256GCM, recip.publicKey, "attack at dawn")
  LET clear AS List OF Byte = crypto::decrypt(crypto::AsymmetricCipher.Ed25519_AES256GCM, recip.privateKey, box)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    let cipher_param = || Parameter {
        name: "cipher",
        desc: "The asymmetric cipher suite to use.",
        aliases: &[],
        ty: ParameterType::named("AsymmetricCipher"),
        default: DefaultValue::None,
    };
    let recip_param = || {
        Parameter {
        name: "recipientPublicKey",
        desc: "The recipient's signing public key for the suite's curve (32-byte Ed25519 for `Ed25519_*`, 57-byte Ed448 for `Ed448_*`); converted to X25519/X448 internally.",
        aliases: &[],
        ty: bytes(),
        default: DefaultValue::None,
    }
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
