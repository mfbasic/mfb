//! `crypto::hmac(type, key, data)` — the unified hash-generic HMAC entry point.
//!
//! Selected by a [`crypto::Hash`] enum (`SHA1`, `SHA2_224`/`SHA2_256`/`SHA2_384`/
//! `SHA2_512`), this member rewrites onto the hash-generic `__crypto_hmac` MFB core (registered by
//! [`super::helper_hmac`]), which computes RFC 2104 HMAC over the `__crypto_shaDigest` /
//! `__crypto_shaBlockSize` dispatch — so every `Hash` variant, present and future, is
//! authenticated by one construction. It is the single HMAC surface (it replaced the
//! former per-digest `hmacSha256`/`hmacSha512` members), mirroring `hash` over `Hash`.
//!
//! Two overloads mirror `hash`. The `List OF Byte` `data` form rewrites to
//! `__crypto_hmac`; the `String` form rewrites to the `__crypto_hmacText` shim
//! (registered by [`super::helper_hmac_text`]) which UTF-8-encodes the string and
//! re-enters the bytes path — a `String` and a `List OF Byte` are not
//! ABI-interchangeable, so the two overloads rewrite to distinct MFB bodies.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Compute the HMAC message authentication code of a message under a key, selected by a `crypto::Hash`."#;
const DESC: &str = r#"`crypto::hmac(type, key, data)` computes the keyed-hash message authentication code
(HMAC) of `data` under the secret `key`, using the hash selected by `type` — a
`crypto::Hash`: `SHA1`, `SHA2_224`/`SHA2_256`/`SHA2_384`/`SHA2_512`, or
`SHA3_224`/`SHA3_256`/`SHA3_384`/`SHA3_512`. It returns the MAC as a raw
`List OF Byte` whose length is the digest length of `type`: 20 bytes for `SHA1`
and 28/32/48/64 for the 224/256/384/512-bit widths of SHA-2 or SHA-3. This one
call is the package's single HMAC surface. (HMAC-SHA1
remains cryptographically sound — HMAC does not rely on collision resistance — but
`Hash.SHA1` still reports the `CRYPTO_SHA1_INSECURE` advisory; prefer `SHA2_256`
unless a peer requires SHA-1.)

A key of any length is accepted. Following the HMAC construction, a key longer than
the hash's block size (64 bytes for `SHA1`/`SHA2_224`/`SHA2_256`, 128 bytes for
`SHA2_384`/`SHA2_512`, and the sponge rate — 144/136/104/72 bytes — for
`SHA3_224`/`SHA3_256`/`SHA3_384`/`SHA3_512`, per FIPS 202 §7) is first hashed down
to the digest length, and a key shorter than
the block size is right-padded with zero bytes to the block size; the padded key is
then combined with the inner (`0x36`) and outer (`0x5c`) pads. The MAC is a
deterministic function of `type`, `key`, and `data` alone — the same inputs always
produce the same bytes, with no salt or randomness. Any key and message length is
accepted, including empty; the function is **total** and never raises an error.

Two overloads accept the message: the `List OF Byte` overload authenticates the raw
bytes as given, while the `String` overload authenticates the string's UTF-8 encoding
(equivalent to passing `strings::toBytes(s)`). The `key` is always raw bytes.

A MAC is raw binary, not text — stringify it with `encoding::hexEncode` or
`encoding::base64Encode` to display or store it. **Always** compare a received MAC
against a freshly computed one with `crypto::constantTimeEqual`, never `=`, so the
check does not leak the position of the first differing byte through timing.

**Implementation.** HMAC is specified by RFC 2104 (equivalently FIPS 198-1), here
layered over the selected hash. The MAC is computed in-process by a portable
MFBASIC software core over the `bits` package — no platform cryptographic library is
called — so the output is **byte-identical on macOS, Linux, and Windows** (and across
aarch64/x86-64). The core is hash-generic over the `Hash` enum, so a future `Hash`
variant is supported without new code."#;
const EX: &str = r#"Authenticate a message under SHA-256 and print the MAC as hex:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET mac AS List OF Byte = crypto::hmac(Hash.SHA2_256, key, message)
  io::print(encoding::hexEncode(mac))
END SUB
```

Verify a received MAC in constant time (the `String` overload hashes the UTF-8 bytes):

```
IMPORT crypto
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(64)
  LET received AS List OF Byte = crypto::hmac(Hash.SHA2_512, key, "payload")
  IF crypto::constantTimeEqual(received, crypto::hmac(Hash.SHA2_512, key, "payload")) THEN
    io::print("authentic")
  ELSE
    io::print("tampered")
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hmac",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("crypto::Hash, List OF Byte, (List OF Byte or String)"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "type",
                        desc: "The hash algorithm underlying the HMAC.",
                        aliases: &[],
                        ty: ParameterType::named("Hash"),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "key",
                        desc: "The secret HMAC key.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "data",
                        desc: "The message bytes to authenticate.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                ],
                return_type: bytes(),
                errors: vec![],
                body: Body::Rewrite("__crypto_hmac"),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "type",
                        desc: "The hash algorithm underlying the HMAC.",
                        aliases: &[],
                        ty: ParameterType::named("Hash"),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "key",
                        desc: "The secret HMAC key.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "data",
                        desc: "A string whose UTF-8 bytes are authenticated.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                ],
                return_type: bytes(),
                errors: vec![],
                body: Body::Rewrite("__crypto_hmacText"),
            },
        ],
    });
}
