//! `crypto::convert(conv, keys)` — convert a key pair between curve encodings.
//!
//! Selected by a [`crypto::KeyConvert`] enum, this member rewrites onto the pure-MFB
//! `__crypto_convert` core (registered by [`super::helper_convert`]) — no platform
//! library, no `AbiFunction`, so (like `hmac`/`hkdf`) it is NOT in any backend's
//! `runtime_calls`. For `Ed25519ToX25519` it converts both halves of an Ed25519
//! `crypto::KeyPair` to the matching X25519 key pair (the birational
//! Edwards→Montgomery map for the public key, `clamp(SHA-512(seed))` for the private),
//! reproducing libsodium's `crypto_sign_ed25519_{pk,sk}_to_curve25519`; for
//! `Ed448ToX448` it applies the RFC 7748 §4.2 isogeny and `SHAKE256(seed)[0..56]`
//! (libdecaf's convention), see [`super::helper_ed448_pub_to_x448`].

use super::{Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str =
    r#"Convert a key pair between curve encodings, selected by a `crypto::KeyConvert`."#;
const DESC: &str = r#"`crypto::convert(conv, keys)` converts the `crypto::KeyPair` `keys` between curve
encodings, as selected by `conv` (a `crypto::KeyConvert`), returning the converted
`crypto::KeyPair`.

**`Ed25519ToX25519`** converts an Ed25519 (RFC 8032) signing key pair to the
matching X25519 / Curve25519 (RFC 7748) ECDH key pair:

- **public key** — the Edwards `y` coordinate is decoded from the 32-byte point
  and mapped to the Montgomery `u` coordinate by the standard birational map
  `u = (1 + y) / (1 - y) mod 2^255 - 19`;
- **private key** — the 32-byte Ed25519 seed is hashed with SHA-512 and the low 32
  bytes are clamped: `X25519 scalar = clamp(SHA-512(seed)[0..32])`.

This reproduces libsodium's `crypto_sign_ed25519_pk_to_curve25519` /
`crypto_sign_ed25519_sk_to_curve25519`, so the converted pair performs the same
ECDH as one from `crypto::generate(crypto::Certificate.X25519)`. It lets a single Ed25519
identity be used for both signing (`crypto::sign`) and encryption
(`crypto::encrypt` / `crypto::decrypt`, which perform this conversion internally).

**`Ed448ToX448`** converts an Ed448 (RFC 8032) signing key pair — a 57-byte seed and
a 57-byte public key, as `crypto::generate(crypto::Certificate.Ed448)` returns — to the
matching X448 / Curve448 (RFC 7748) ECDH key pair of 56-byte keys:

- **public key** — the Edwards `y` coordinate (the low 56 bytes) is mapped to the
  Montgomery `u` coordinate by the RFC 7748 §4.2 edwards448→curve448 4-isogeny
  `u = y² / x²`, computed as `y²·(1 − d·y²) / (1 − y²)` with `d = −39081` (no square
  root is needed) — the map libdecaf's `decaf_ed448_convert_public_key_to_x448`
  uses; it sends the edwards448 base point to `u = 5`;
- **private key** — the first 56 bytes of `SHAKE256(seed, 114)`, i.e. the Ed448
  secret-scalar bytes before pruning (libdecaf's
  `decaf_ed448_convert_private_key_to_x448`); RFC 8032's pruning and RFC 7748's
  scalar clamp coincide on those bytes, so `exchange(X448, privateKey, basepoint)`
  reproduces the converted `publicKey` exactly.

A pair whose halves are not both 57 bytes (`Ed448ToX448`) or both 32 bytes
(`Ed25519ToX25519`) raises `ErrInvalidArgument` — so handing an Ed448 pair to the
Ed25519 map, or vice versa, is rejected rather than silently mis-mapped.

**No curve tagging.** A `crypto::KeyPair` carries no tag identifying its curve, so
`convert` cannot detect a mismatched input beyond the length check — a 32-byte pair
that is not really an Ed25519 pair (or a 57-byte one that is not an Ed448 pair) is
simply mapped, producing an incorrect result rather than an error, so make sure
`keys` really is a key pair of the source curve.

**Key reuse note.** Sharing one key pair across both signing and Diffie-Hellman is a
deliberate, supported convenience here, but reusing key material across primitives
is generally discouraged; prefer separate keys when your threat model allows.

**Implementation.** The curve maps, SHA-512, SHAKE256, and scalar clamping are pure
MFBASIC software cores computed over the `bits` package — no platform cryptographic
library — so the converted key pair is byte-identical on every target (macOS, Linux,
Windows; aarch64, x86-64). The Ed448 conversion is verified against an
OpenSSL-backed oracle through the invariant `X448(convertedPrivate, 5) ==
convertedPublic`."#;
const EX: &str = r#"```
IMPORT crypto

SUB main()
  LET ed AS crypto::KeyPair = crypto::generate(crypto::Certificate.Ed25519)
  LET x AS crypto::KeyPair = crypto::convert(crypto::KeyConvert.Ed25519ToX25519, ed)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "convert",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "conv",
                    desc: "The key-conversion to perform.",
                    aliases: &[],
                    ty: ParameterType::named("KeyConvert"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "keys",
                    desc: "The key pair to convert.",
                    aliases: &[],
                    ty: ParameterType::named("KeyPair"),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named("KeyPair"),
            errors: vec![],
            body: Body::Rewrite("__crypto_convert"),
        }],
    });
}
