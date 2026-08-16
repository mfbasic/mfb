//! `crypto::ed25519Sign` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). A single-overload SOURCE member that
//! signs a message with an Ed25519 private key and returns a `List OF Byte`.
//! Docs migrated from `src/docs/man/builtins/crypto/ed25519Sign.md`.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, RegistryFunction};

const INTRO: &str = r#"Sign a message with an Ed25519 private key (RFC 8032)."#;
const DESC: &str = r#"`crypto::ed25519Sign` produces an Ed25519 signature over `message` using
`privateKey`, following RFC 8032 (PureEdDSA over Curve25519). The result is a
fixed 64-byte signature returned as a `List OF Byte`, the concatenation of the
32-byte `R` point and the 32-byte `S` scalar. Verify it later with
`crypto::ed25519Verify` given the matching public key.

`privateKey` is the 32-byte Ed25519 secret seed — exactly the `privateKey` field
returned by `crypto::generateEd25519`. The public key, nonce prefix, and signing
scalar are all derived from this seed by SHA-512, so no separate public key is
passed in. `message` is the raw bytes to sign; Ed25519 is a PureEdDSA scheme, so
the whole message is signed directly with no pre-hashing required from the
caller.

Ed25519 signing is deterministic: the per-signature nonce is derived from the
key and the message rather than from randomness, so signing the same
`(privateKey, message)` always yields the same 64-byte signature. This holds on
every target (macOS/Linux, aarch64/x86-64), since Ed25519 is a portable software
core with byte-identical output and uses no platform crypto library.

**Secret safety.** `privateKey` is sensitive secret material. Anyone who holds
it can forge signatures. Never log it, and treat `typeName` / `toString` /
diagnostics as non-security boundaries. To store or display a signature,
stringify its bytes with `encoding::hexEncode` (lowercase hex) or
`encoding::base64Encode`."#;
const EX: &str = r#"Generate a key, sign a message, and verify the signature:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET kp AS crypto::KeyPair = crypto::generateEd25519()
  LET sig AS List OF Byte = crypto::ed25519Sign(kp.privateKey, message)
  LET ok AS Boolean = crypto::ed25519Verify(kp.publicKey, message, sig)
END SUB
```

Display a signature as hex:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET kp AS crypto::KeyPair = crypto::generateEd25519()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::ed25519Sign(kp.privateKey, message)
  io::print(encoding::hexEncode(sig))
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "ed25519Sign",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "privateKey",
                    desc: "The Ed25519 private key bytes.",
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
            body: Body::Rewrite("__crypto_ed25519Sign"),
        }],
    });
}
