//! `crypto::p256Sign` — descriptor entry + authored docs.
//!
//! A NATIVE member: ECDSA signing on the NIST P-256 curve. Its `Body::native`
//! OS-seam slots point at [`super::native::lower_crypto_ec`], the shared elliptic-curve
//! lowering. Docs migrated from `src/docs/man/builtins/crypto/p256Sign.md`.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, RegistryFunction};

const INTRO: &str = r#"ECDSA-sign a message with a NIST P-256 private key (FIPS 186)."#;
const DESC: &str = r#"`crypto::p256Sign` produces an ECDSA signature over `message` using `privateKey`
on the NIST P-256 curve (FIPS 186), hashing the message with SHA-256 internally.
The result is an ASN.1 DER `Ecdsa-Sig-Value` (X9.62) returned as a `List OF Byte`.
Verify it later with `crypto::p256Verify` given the matching public key.

`privateKey` is the 97-byte wire form `0x04 || X || Y || K` — the 65-byte
uncompressed point (`0x04` prefix plus the two 32-byte field elements `X` and `Y`)
followed by the 32-byte secret scalar `K`. This is exactly the `privateKey` field
returned by `crypto::generateP256`. `message` is the raw bytes to sign; it is
hashed with SHA-256 as part of the platform signing call, so the caller does not
pre-hash it. The DER-encoded signature is variable length (roughly 70–72 bytes),
since the encoding of the two integers `r` and `s` depends on their leading bits.

The NIST curves bind the platform key API: `SecKeyCreateSignature` with
`kSecKeyAlgorithmECDSASignatureMessageX962SHA256` on macOS, and OpenSSL
`EVP_PKEY` signing on Linux. The DER signature is wire-compatible across
platforms and with OpenSSL / pyca. Unlike Ed25519, ECDSA signing is
**non-deterministic**: a fresh random nonce is drawn per call, so signing the same
`(privateKey, message)` twice yields two different signatures. Both verify
correctly.

**Secret safety.** `privateKey` embeds the secret scalar `K`. Anyone who holds it
can forge signatures. Never log it, and treat `typeName` / `toString` /
diagnostics as non-security boundaries. To store or display a signature, stringify
its bytes with `encoding::hexEncode` (lowercase hex) or `encoding::base64Encode`."#;
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

Display a signature as hex:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET kp AS crypto::KeyPair = crypto::generateP256()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::p256Sign(kp.privateKey, message)
  io::print(encoding::hexEncode(sig))
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "p256Sign",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "privateKey",
                    desc: "The P-256 private key bytes.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "message",
                    desc: "The message bytes to sign.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
            ],
            return_type: bytes(),
            errors: vec!["ErrInvalidArgument"],
            body: Body::native(
                Some(super::native::lower_crypto_ec),
                Some(super::native::lower_crypto_ec),
                None,
            ),
        }],
    });
}
