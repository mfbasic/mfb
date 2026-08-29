//! `crypto::exchange(type, privateKey, publicKey)` — Diffie-Hellman key agreement.
//!
//! Selected by a [`crypto::Certificate`] enum, restricted to the key-agreement
//! members `X25519` (RFC 7748 §5, 32-byte keys) and `X448` (RFC 7748 §5, 56-byte
//! keys). Pure-MFB rewrite onto `__crypto_exchange` (registered by
//! [`super::helper_exchange`]) — no platform library, no `AbiFunction`, so (like
//! `convert`) it is NOT in any backend's `runtime_calls`. It is the public face of
//! the `__crypto_x25519` / `__crypto_x448` ladders the HPKE KEM builds on.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Compute a Diffie-Hellman shared secret from your private key and a peer's public key (X25519 or X448)."#;
const DESC: &str = r#"`crypto::exchange(type, privateKey, publicKey)` performs elliptic-curve
Diffie-Hellman for the key-agreement curve selected by `type` — `Certificate.X25519`
(RFC 7748 X25519, 32-byte keys, 32-byte secret) or `Certificate.X448` (RFC 7748
X448, 56-byte keys, 56-byte secret) — combining your `privateKey` with the peer's
`publicKey` and returning the raw shared secret as a `List OF Byte`. Both parties
compute the same bytes: `exchange(t, a.privateKey, b.publicKey)` equals
`exchange(t, b.privateKey, a.publicKey)` for two pairs from
`crypto::generate(t)` (or from `crypto::convert`).

The raw secret is a uniformly-distributed-looking field element, **not** a key:
derive symmetric keys from it with `crypto::hkdf` (binding the two public keys and a
context label into `info`), or use `crypto::encrypt`/`crypto::decrypt`, which do this
for you per RFC 9180.

**Fail closed.** `type` must be `X25519` or `X448` — a signing certificate (`P256`,
`P384`, `P521`, `Ed25519`, `Ed448`) raises `ErrInvalidArgument`; so does a key of the
wrong length for the curve. If the computed secret is all zeros — which happens when
the peer's public key is a low-order point (RFC 7748 §6.1 requires this check) —
`exchange` raises `ErrInvalidArgument` instead of returning a secret an attacker could
force. The private key is clamped internally per RFC 7748, so a raw random scalar and
a generated key behave identically.

**Implementation.** Both ladders are portable MFBASIC software cores over the
`bits` package (X25519 over the shared GF(2^255−19) arithmetic, X448 over a 16 ×
28-bit-limb GF(2^448−2^224−1) field): a fixed 255-/448-iteration Montgomery ladder
whose conditional swap is branch-free, so the output is byte-identical on every
target and no control flow depends on the private key. Verified against the RFC 7748
§5.2 and §6 vectors."#;
const EX: &str = r#"Two parties agree on a secret over X448 and derive a key from it:

```
IMPORT crypto
IMPORT strings
IMPORT io

SUB main()
  LET alice AS crypto::KeyPair = crypto::generate(Certificate.X448)
  LET bob AS crypto::KeyPair = crypto::generate(Certificate.X448)
  LET s1 AS List OF Byte = crypto::exchange(Certificate.X448, alice.privateKey, bob.publicKey)
  LET s2 AS List OF Byte = crypto::exchange(Certificate.X448, bob.privateKey, alice.publicKey)
  io::print(toString(crypto::constantTimeEqual(s1, s2)))
  LET key AS List OF Byte = crypto::hkdf(Hash.SHA2_256, s1, [], strings::toBytes("demo v1"), 32)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "exchange",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("crypto::Certificate, List OF Byte, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "type",
                    desc: "The key-agreement curve: `Certificate.X25519` or `Certificate.X448`.",
                    aliases: &[],
                    ty: ParameterType::named("Certificate"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "privateKey",
                    desc: "Your private key (32 bytes for X25519, 56 for X448).",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "publicKey",
                    desc: "The peer's public key (32 bytes for X25519, 56 for X448).",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
            ],
            return_type: bytes(),
            errors: vec!["ErrInvalidArgument"],
            body: Body::Rewrite("__crypto_exchange"),
        }],
    });
}
