//! `crypto::generateEd25519` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). A single-overload SOURCE member that
//! takes no arguments and returns a `crypto::KeyPair`. Docs migrated from
//! `src/docs/man/builtins/crypto/generateEd25519.md`.

use super::{Body, Implementation, ParameterType, RegistryFunction};

const INTRO: &str = r#"Generate a random Ed25519 signing key pair (RFC 8032)."#;
const DESC: &str = r#"`crypto::generateEd25519` creates a fresh Ed25519 signing key pair for use with
`crypto::ed25519Sign` and `crypto::ed25519Verify`, following RFC 8032 (PureEdDSA
over Curve25519). It takes no arguments and returns a `crypto::KeyPair` record
with two fields:

- `privateKey` — the 32-byte Ed25519 secret seed (`List OF Byte`).
- `publicKey` — the 32-byte Ed25519 public key (`List OF Byte`), derived from the
  seed by SHA-512, scalar clamping, and scalar-base multiplication.

The secret seed is drawn from the OS CSPRNG via `crypto::randomBytes(32)`, so the
result is random and non-reproducible: every call yields a different key pair.
There is no seeded or deterministic form; to persist a key, store the returned
bytes yourself. Because the count `32` is fixed and valid, the internal
`randomBytes` call never fails on a bad argument, but it can still surface an OS
entropy failure (`ErrUnknown`) or an allocation failure (`ErrOutOfMemory`); the
public-key derivation allocates its own byte lists and can likewise raise
`ErrOutOfMemory`.

Ed25519 is a portable software core, so keys and the algorithm behave identically
on every target (macOS/Linux, aarch64/x86-64) and use no platform crypto library.

**Secret safety.** The `privateKey` field is sensitive secret material. Anyone who
holds it can forge signatures. Never log a `KeyPair`, and treat `typeName` /
`toString` / diagnostics as non-security boundaries. The `publicKey` is safe to
share; distribute it to verifiers.

To display or store a key, stringify its bytes with `encoding::hexEncode`
(lowercase hex) or `encoding::base64Encode`."#;
const EX: &str = r#"Generate a key pair and print the public key as hex:

```
IMPORT crypto
IMPORT encoding
IMPORT io

SUB main()
  LET kp AS crypto::KeyPair = crypto::generateEd25519()
  io::print(encoding::hexEncode(kp.publicKey))
END SUB
```

Sign a message with the freshly generated key:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET kp AS crypto::KeyPair = crypto::generateEd25519()
  LET sig AS List OF Byte = crypto::ed25519Sign(kp.privateKey, message)
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "generateEd25519",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Named("KeyPair"),
            errors: vec![],
            body: Body::Rewrite("__crypto_generateEd25519"),
        }],
    });
}
