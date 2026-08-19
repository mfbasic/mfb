//! `crypto::convert(conv, keys)` — convert a key pair between curve encodings.
//!
//! Selected by a [`crypto::KeyConvert`] enum, this member rewrites onto the pure-MFB
//! `__crypto_convert` core (registered by [`super::helper_convert`]) — no platform
//! library, no `AbiFunction`, so (like `hmac`/`hkdf`) it is NOT in any backend's
//! `runtime_calls`. For `Ed25519ToX25519` it converts both halves of an Ed25519
//! `crypto::KeyPair` to the matching X25519 key pair (the birational
//! Edwards→Montgomery map for the public key, `clamp(SHA-512(seed))` for the private),
//! reproducing libsodium's `crypto_sign_ed25519_{pk,sk}_to_curve25519`.

use super::{Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str =
    r#"Convert a key pair between curve encodings, selected by a `crypto::KeyConvert`."#;
const DESC: &str = r#"`crypto::convert(conv, keys)` converts the `crypto::KeyPair` `keys` between
curve encodings, as selected by `conv` (a `crypto::KeyConvert`), returning the
converted `crypto::KeyPair`.

`Ed25519ToX25519` converts an Ed25519 signing key pair to the matching X25519
(Curve25519 ECDH) key pair: the public key is mapped through the birational
Edwards→Montgomery map `u = (1 + y) / (1 - y)`, and the private key (the 32-byte
Ed25519 seed) is mapped to `clamp(SHA-512(seed)[0..32])`. This mirrors libsodium's
`crypto_sign_ed25519_pk_to_curve25519` / `crypto_sign_ed25519_sk_to_curve25519`, so
a converted key pair performs the same ECDH as one from `generate(Certificate.X25519)`.

The conversion is a portable software core, so its output is **byte-identical on
every target** and uses no platform crypto library."#;
const EX: &str = r#"```
IMPORT crypto

SUB main()
  LET ed AS crypto::KeyPair = crypto::generate(Certificate.Ed25519)
  LET x AS crypto::KeyPair = crypto::convert(KeyConvert.Ed25519ToX25519, ed)
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
                    ty: ParameterType::Named("KeyConvert"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "keys",
                    desc: "The key pair to convert.",
                    aliases: &[],
                    ty: ParameterType::Named("KeyPair"),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Named("KeyPair"),
            errors: vec![],
            body: Body::Rewrite("__crypto_convert"),
        }],
    });
}
