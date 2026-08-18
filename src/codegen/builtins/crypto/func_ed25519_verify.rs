//! `crypto::ed25519Verify` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). A single-overload SOURCE member that
//! checks an Ed25519 signature over a message and returns a `Boolean`.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Verify an Ed25519 signature over a message with a public key (RFC 8032)."#;
const DESC: &str = r#"`crypto::ed25519Verify` checks whether `signature` is a valid Ed25519 signature
of `message` under `publicKey`, following RFC 8032 (PureEdDSA over Curve25519).
It returns `TRUE` if and only if the signature verifies for that exact key and
message, and `FALSE` otherwise. Verification depends only on the three inputs;
the matching signing key is not required.

`publicKey` is the 32-byte Ed25519 public key — exactly the `publicKey` field
returned by `crypto::generateEd25519`. `message` is the raw message bytes that
were signed; Ed25519 is a PureEdDSA scheme, so the whole message is hashed
internally and no pre-hashing is applied by the caller. `signature` is the
64-byte signature produced by `crypto::ed25519Sign` or any interoperating
implementation — the concatenation of the 32-byte `R` point and the 32-byte `S`
scalar.

Verification is total and never raises: it always returns a `TRUE`/`FALSE`
verdict. A `publicKey` that is not exactly 32 bytes, a `signature` that is not
exactly 64 bytes, a public key that does not decode to a valid curve point, a
signature whose `S` scalar is not canonical (`S >= L`, the group order — such a
signature is malleable and is rejected so the signature bytes remain a stable
identity), or a signature that simply does not match all return `FALSE` — a
failed verdict is a normal outcome, not an error.

Verification is deterministic and platform-independent: the same
`(publicKey, message, signature)` triple yields the same verdict on every target
(macOS/Linux, aarch64/x86-64), because Ed25519 is a portable software core with
byte-identical behavior and uses no platform crypto library. Signatures
interoperate across platforms and with standard toolkits.

The final comparison of the recomputed `R` point against the signature's `R` is
done with a constant-time byte compare, so a matching-length verification does
not leak timing information about how far the two points agree."#;
const EX: &str = r#"Generate a key, sign a message, and verify the signature:

```
IMPORT crypto
IMPORT strings
IMPORT io

SUB main()
  LET kp AS crypto::KeyPair = crypto::generateEd25519()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::ed25519Sign(kp.privateKey, message)
  LET ok AS Boolean = crypto::ed25519Verify(kp.publicKey, message, sig)
  io::print(toString(ok))
END SUB
```

A tampered message fails verification (returns `FALSE`, not an error):

```
IMPORT crypto
IMPORT strings
IMPORT io

SUB main()
  LET kp AS crypto::KeyPair = crypto::generateEd25519()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::ed25519Sign(kp.privateKey, message)
  LET altered AS List OF Byte = strings::toBytes("attack at dusk")
  LET bad AS Boolean = crypto::ed25519Verify(kp.publicKey, altered, sig)
  io::print(toString(bad))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "ed25519Verify",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "publicKey",
                    desc: "The Ed25519 public key bytes.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "message",
                    desc: "The message bytes that were signed.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "signature",
                    desc: "The signature bytes to verify.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::Rewrite("__crypto_ed25519Verify"),
        }],
    });
}
