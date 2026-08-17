//! `crypto::generateP384` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). A single-overload SOURCE member that
//! takes no arguments and returns a `crypto::KeyPair`. Docs migrated from
//! `src/docs/man/builtins/crypto/generateP384.md`.

use super::{Body, Implementation, ParameterType, RegistryFunction};

const INTRO: &str = r#"Generate a random NIST P-384 ECDSA key pair (FIPS 186)."#;
const DESC: &str = r#"`crypto::generateP384` creates a fresh ECDSA key pair over the NIST P-384 curve
(FIPS 186) for use with `crypto::p384Sign` and `crypto::p384Verify`. It takes no
arguments and returns a `crypto::KeyPair` record with two fields:

- `privateKey` — 145 bytes, the wire form `0x04 || X || Y || K`: the SEC1
  uncompressed public point (`0x04` tag, 48-byte `X`, 48-byte `Y`) followed by
  the 48-byte big-endian private scalar `K`. It is self-contained and is what
  `crypto::p384Sign` consumes.
- `publicKey` — 97 bytes, the wire form `0x04 || X || Y`: the leading SEC1
  uncompressed public point, sliced from the private bytes.

The key is produced by a native raw keygen helper that binds the platform key
API — `SecKey` on macOS, `EVP_PKEY`/`EC_KEY` on Linux (OpenSSL) — while the
public/private wire encodings above are identical across macOS and Linux and are
interoperable: a key produced on one platform is accepted on the other and by
OpenSSL/pyca.

The secret scalar is drawn from the platform CSPRNG, so the result is random and
non-reproducible: every call yields a different key pair. There is no seeded or
deterministic form; to persist a key, store the returned bytes yourself.

**Secret safety.** The `privateKey` field embeds the secret scalar `K`. Anyone
who holds it can forge signatures. Never log a `KeyPair`, and treat `typeName` /
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
  LET kp AS crypto::KeyPair = crypto::generateP384()
  io::print(encoding::hexEncode(kp.publicKey))
END SUB
```

Sign a message with the freshly generated key:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET kp AS crypto::KeyPair = crypto::generateP384()
  LET sig AS List OF Byte = crypto::p384Sign(kp.privateKey, message)
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "generateP384",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Named("KeyPair"),
            errors: vec![],
            body: Body::Rewrite("__crypto_generateP384"),
        }],
    });
}
