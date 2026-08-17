//! `crypto::p256Verify` — descriptor entry + authored docs.
//!
//! A NATIVE member: ECDSA verification on the NIST P-256 curve. Its `Body::native`
//! OS-seam slots point at [`super::native::lower_crypto_ec`], the shared elliptic-curve
//! lowering. Docs migrated from `src/docs/man/builtins/crypto/p256Verify.md`.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Verify an ECDSA P-256/SHA-256 signature against a public key (FIPS 186)."#;
const DESC: &str = r#"`crypto::p256Verify` checks whether `signature` is a valid ECDSA signature of
`message` under `publicKey` on the NIST P-256 curve with SHA-256 (FIPS 186). It
returns `TRUE` if and only if the signature verifies for that exact key and
message, and `FALSE` otherwise. The message is hashed with SHA-256 internally, so
pass the raw message bytes, not a digest.

`publicKey` is the 65-byte SEC1 uncompressed point `0x04 || X || Y`, where `X` and
`Y` are the two 32-byte big-endian affine coordinates — exactly the `publicKey`
field returned by `crypto::generateP256`. `signature` is an ASN.1 DER
`Ecdsa-Sig-Value` (X9.62), as produced by `crypto::p256Sign`. Verification depends
only on the three inputs; the private signing key is not required.

A failed verdict is distinguished from a malformed key. A valid-length public key
paired with a signature that simply does not match returns `FALSE` — a normal
outcome, not an error. But a `publicKey` that is not a well-formed 65-byte P-256
SEC1 point (wrong length, or bytes that do not decode to a valid curve point)
raises `ErrInvalidArgument` rather than returning a verdict. A malformed
`signature` that the platform cannot parse also verifies as `FALSE`.

Verification is total and platform-independent: the same
`(publicKey, message, signature)` triple yields the same verdict on macOS and
Linux, on aarch64 and x86-64. The NIST curves bind the platform key API —
`SecKeyVerifySignature` with `kSecKeyAlgorithmECDSASignatureMessageX962SHA256` on
macOS, and OpenSSL `EVP_DigestVerify` on Linux. Keys and DER signatures are
wire-compatible across platforms and with OpenSSL / pyca, so a signature made on
one system verifies on another. ECDSA signing is non-deterministic (a fresh nonce
per call), but a signature and its verdict do not depend on that nonce."#;
const EX: &str = r#"Generate a key, sign a message, and verify the signature:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET kp AS crypto::KeyPair = crypto::generateP256()
  LET sig AS List OF Byte = crypto::p256Sign(kp.privateKey, message)
  LET ok AS Boolean = crypto::p256Verify(kp.publicKey, message, sig)
END SUB
```

A tampered message fails verification (returns `FALSE`, not an error):

```
IMPORT crypto
IMPORT strings

SUB main()
  LET kp AS crypto::KeyPair = crypto::generateP256()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::p256Sign(kp.privateKey, message)
  LET altered AS List OF Byte = strings::toBytes("attack at dusk")
  LET ok AS Boolean = crypto::p256Verify(kp.publicKey, altered, sig)
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "p256Verify",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "publicKey",
                    desc: "The P-256 public key bytes.",
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
                    desc: "The DER-encoded signature bytes to verify.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::native(
                Some(super::native::lower_crypto_ec),
                Some(super::native::lower_crypto_ec),
                None,
            ),
        }],
    });
}
