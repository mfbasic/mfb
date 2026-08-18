//! `crypto::generateP521` — descriptor entry + authored docs.
//!
//! A NATIVE member that takes no arguments and returns a `crypto::KeyPair`. Its
//! `Body::native` OS-seam slots point at [`super::native::lower_crypto_ec`], which
//! generates the P-521 key and **builds the `KeyPair` record directly** via the
//! generic spec-canonical record marshaller — collapsing the former
//! `generateP521Raw` native helper plus its `__crypto_generateP521` source glue
//! into a single member (see [`super::func_generate_p256`] for the mechanism).
//! Infallible surface (`errors: vec![]`): callers invoke it bare, as before.

use super::{Body, Implementation, ParameterType, RegistryFunction};

const INTRO: &str = r#"Generate a random NIST P-521 ECDSA key pair (FIPS 186)."#;
const DESC: &str = r#"`crypto::generateP521` creates a fresh ECDSA key pair over the NIST P-521 curve
(FIPS 186) for use with `crypto::p521Sign` and `crypto::p521Verify`. It takes no
arguments and returns a `crypto::KeyPair` record with two fields:

- `privateKey` — 199 bytes, the wire form `0x04 || X || Y || K`: the SEC1
  uncompressed public point (`0x04` tag, 66-byte `X`, 66-byte `Y`) followed by
  the 66-byte big-endian private scalar `K`. It is self-contained and is what
  `crypto::p521Sign` consumes.
- `publicKey` — 133 bytes, the wire form `0x04 || X || Y`: the leading SEC1
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
  LET kp AS crypto::KeyPair = crypto::generateP521()
  io::print(encoding::hexEncode(kp.publicKey))
END SUB
```

Sign a message with the freshly generated key:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET kp AS crypto::KeyPair = crypto::generateP521()
  LET sig AS List OF Byte = crypto::p521Sign(kp.privateKey, message)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "generateP521",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Named("KeyPair"),
            errors: vec![],
            body: Body::native(
                Some(super::native::lower_crypto_ec),
                Some(super::native::lower_crypto_ec),
                None,
            ),
        }],
    });
}
