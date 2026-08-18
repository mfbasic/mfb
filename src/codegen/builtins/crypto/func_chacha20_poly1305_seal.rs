//! `crypto::chacha20Poly1305Seal` — descriptor entry + authored docs.
//!
//! An AEAD source member: the ciphertext-and-tag sealing half of ChaCha20-Poly1305
//! (RFC 8439). Its `Body::Rewrite("__crypto_chacha20Poly1305Seal")` repoints the
//! citation at the `package.mfb` helper; the optional `aad` parameter fills to the
//! empty byte list.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Encrypt and authenticate a message with the ChaCha20-Poly1305 AEAD construction (RFC 8439)."#;
const DESC: &str = r#"`crypto::chacha20Poly1305Seal` encrypts and authenticates `plaintext` with the
ChaCha20-Poly1305 AEAD construction, as specified by RFC 8439. It returns a
`crypto::Sealed` record holding the ciphertext (the same length as `plaintext`)
and a 16-byte Poly1305 authentication tag that binds the ciphertext together with
any additional authenticated data. The tag is later checked by
`crypto::chacha20Poly1305Open`, which fails closed on any mismatch.

`key` must be exactly 32 bytes (a 256-bit key) and `nonce` must be exactly 12
bytes (the 96-bit RFC 8439 nonce); any other length raises `ErrInvalidArgument`.
The optional `aad` (additional authenticated data) is authenticated but not
encrypted: it is covered by the tag yet absent from the ciphertext, so a receiver
must supply the identical `aad` to `crypto::chacha20Poly1305Open`. `aad` defaults
to the empty list when omitted. `plaintext` may be empty, in which case the
result carries an empty ciphertext and a tag over the `aad` alone.

The cipher is a portable software core computed over the `bits` package, so its
output is **byte-identical on every target** (macOS/Linux, aarch64/x86-64) and
uses no platform crypto library. ChaCha20-Poly1305 is a strong choice on targets
without AES hardware acceleration; AES-256-GCM (`crypto::aes256GcmSeal`) is the
interchangeable alternative.

Nonce uniqueness is mandatory. ChaCha20-Poly1305 is catastrophically insecure if
a `(key, nonce)` pair is ever reused: repeating a nonce under the same key leaks
the XOR of the plaintexts and can expose the Poly1305 authentication key, breaking
both confidentiality and integrity. Never reuse a `(key, nonce)` pair — generate a
fresh nonce for every message with `crypto::randomBytes(12)` and store or transmit
it alongside the ciphertext (the nonce is not secret)."#;
const EX: &str = r#"Seal a message with a fresh random nonce:

```
IMPORT crypto
IMPORT encoding
IMPORT strings
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
  LET plaintext AS List OF Byte = strings::toBytes("attack at dawn")
  LET box AS crypto::Sealed = crypto::chacha20Poly1305Seal(key, nonce, plaintext)

  io::print(encoding::hexEncode(box.ciphertext))
  io::print(encoding::hexEncode(box.tag))
END SUB
```

Seal with additional authenticated data (a header), then open it:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
  LET plaintext AS List OF Byte = strings::toBytes("attack at dawn")
  LET header AS List OF Byte = strings::toBytes("v1")
  LET box AS crypto::Sealed = crypto::chacha20Poly1305Seal(key, nonce, plaintext, header)
  LET clear AS List OF Byte = crypto::chacha20Poly1305Open(key, nonce, box.ciphertext, box.tag, header)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "chacha20Poly1305Seal",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "key",
                    desc: "The 32-byte AES-256 key.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "nonce",
                    desc: "The 12-byte nonce; must be unique per key.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "plaintext",
                    desc: "The message bytes to encrypt.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "aad",
                    desc: "Optional additional authenticated data; defaults to empty.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::Fill {
                        type_name: bytes(),
                        expr: "",
                    },
                },
            ],
            return_type: ParameterType::Named("Sealed"),
            errors: vec!["ErrInvalidArgument"],
            body: Body::Rewrite("__crypto_chacha20Poly1305Seal"),
        }],
    });
}
