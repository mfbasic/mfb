//! `crypto::seal(cipher, key, nonce, data[, aad])` — the unified `AbiFunction` AEAD
//! sealing entry point.
//!
//! Selected by a [`crypto::SymmetricCipher`] enum (`AES256GCM`/`CHACHA20POLY1305`), this
//! member's `Body::abi_function` body branches on the enum ordinal and dispatches to the
//! matching always-emitted MFB software AEAD `Seal` core, exactly how `func_sign`'s
//! Ed25519 branch routes to `#crypto_ed25519Sign`. The AEAD math stays in MFB — this file
//! only consolidates the two `*Seal` members behind one `SymmetricCipher`-selected surface
//! (mirroring `hash` over `Hash`).
//!
//! Two overloads mirror the per-cipher `*Seal` members. The `List OF Byte` form is the
//! `AbiFunction` proper: it branches on the ordinal and calls the `Seal` cores. The
//! `String` form is a `Body::Rewrite` to the `__crypto_sealText` MFB shim (registered by
//! [`super::helper_seal_text`]) which UTF-8-encodes the string and re-enters the bytes
//! `seal` — so a `String` `data` argument reaches the same per-ordinal dispatch (identical
//! to the hash `_text` shim), and the two overloads never share the one `crypto.seal`
//! runtime symbol (an `AbiFunction` member emits exactly one helper, and
//! `abi_function_lower` binds the first overload's body; a second `AbiFunction` overload
//! would silently reuse that same `List OF Byte`-shaped body on a `String` pointer). The
//! shared per-ordinal dispatch scaffolding lives in [`super::gen_cipher`].
//!
//! `aad` is a trailing optional parameter filling to the empty byte list.

use super::gen_cipher;
use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// The `AbiFunction` body for the `List OF Byte` overload of `crypto::seal`. `args[0]` is
/// the `SymmetricCipher` ordinal; `args[1..=4]` are the `key`, `nonce`, `data`, and `aad`
/// collection pointers in their argument registers. Branches on the ordinal to the
/// matching MFB software AEAD `Seal` core, which leaves the `Sealed` result in the result
/// registers — so this is a self-managed fallible ABI body that returns the `void`
/// sentinel (the wrapper adds no epilogue). The `String` `data` overload reaches the
/// equivalent path through the `__crypto_sealText` shim, so this body only ever runs over
/// a `List OF Byte` `data`.
pub(crate) fn lower_seal(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ord = args[0].location.clone();
    // Core takes (key, nonce, plaintext, aad).
    let arg_ops = [
        args[1].location.clone(),
        args[2].location.clone(),
        args[3].location.clone(),
        args[4].location.clone(),
    ];

    let done = format!("{symbol}_done");
    gen_cipher::emit_dispatch(
        builder,
        &symbol,
        &ord,
        &arg_ops,
        gen_cipher::Op::Seal,
        ctx,
        &done,
    )?;

    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);

    Ok(ValueResult {
        type_: "Sealed".to_string(),
        location: Operand::from("void"),
        text: "crypto.seal".to_string(),
    })
}

const INTRO: &str = r#"Encrypt and authenticate a message with an AEAD cipher, selected by a `crypto::SymmetricCipher`."#;
const DESC: &str = r#"`crypto::seal(cipher, key, nonce, data)` encrypts and authenticates `data` with
the authenticated cipher selected by `cipher` (a `crypto::SymmetricCipher`:
`AES256GCM` or `CHACHA20POLY1305`), and returns a `crypto::Sealed` record holding
the ciphertext (the same length as `data`) and a 16-byte authentication tag. It is
the unified front door for the two symmetric AEAD ciphers behind one
`SymmetricCipher`-selected call, later verified and decrypted by
`crypto::open(cipher, …)`.

`key` must be exactly 32 bytes (a 256-bit key) and `nonce` must be exactly 12
bytes; any other length raises `ErrInvalidArgument`. The optional `aad` (additional
authenticated data) is authenticated but not encrypted: it is covered by the tag
yet absent from the ciphertext, so a receiver must supply the identical `aad` to
`crypto::open`. `aad` defaults to the empty list when omitted. `data` may be empty.

The ciphers are portable software cores computed over the `bits` package, so their
output is **byte-identical on every target** (macOS/Linux/Windows, aarch64/x86-64)
and use no platform crypto library. The `List OF Byte` overload seals the raw bytes
as given; the `String` overload seals the string's UTF-8 encoding.

Nonce uniqueness is mandatory. AEAD ciphers are catastrophically insecure if a
`(key, nonce)` pair is ever reused — generate a fresh nonce for every message with
`crypto::randomBytes(12)` and store or transmit it alongside the ciphertext (the
nonce is not secret)."#;
const EX: &str = r#"Seal a message and open it back:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET secret AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
  LET box AS crypto::Sealed = crypto::seal(SymmetricCipher.AES256GCM, secret, nonce, "attack at dawn")
  LET clear AS List OF Byte = crypto::open(SymmetricCipher.AES256GCM, secret, nonce, box)
END SUB
```

Seal raw bytes with additional authenticated data (a header):

```
IMPORT crypto
IMPORT strings

SUB main()
  LET secret AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
  LET plaintext AS List OF Byte = strings::toBytes("attack at dawn")
  LET header AS List OF Byte = strings::toBytes("v1;msg-42")
  LET box AS crypto::Sealed = crypto::seal(SymmetricCipher.CHACHA20POLY1305, secret, nonce, plaintext, header)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    let cipher_param = || Parameter {
        name: "cipher",
        desc: "The AEAD cipher to use.",
        aliases: &[],
        ty: ParameterType::Named("SymmetricCipher"),
        default: DefaultValue::None,
    };
    let key_param = || Parameter {
        name: "key",
        desc: "The 32-byte symmetric key; used for both seal and open.",
        aliases: &[],
        ty: bytes(),
        default: DefaultValue::None,
    };
    let nonce_param = || Parameter {
        name: "nonce",
        desc: "The 12-byte nonce; must be unique per key.",
        aliases: &[],
        ty: bytes(),
        default: DefaultValue::None,
    };
    let aad_param = || Parameter {
        name: "aad",
        desc: "Optional additional authenticated data; defaults to empty.",
        aliases: &[],
        ty: bytes(),
        default: DefaultValue::Fill {
            type_name: bytes(),
            expr: "",
        },
    };
    pkg.add_function(RegistryFunction {
        name: "seal",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some(
            "crypto::SymmetricCipher, List OF Byte, List OF Byte, List OF Byte or String[, List OF Byte]",
        ),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    cipher_param(),
                    key_param(),
                    nonce_param(),
                    Parameter {
                        name: "data",
                        desc: "The message bytes to encrypt. Any length is accepted, including the empty list.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    aad_param(),
                ],
                return_type: ParameterType::Named("Sealed"),
                errors: vec!["ErrInvalidArgument"],
                body: Body::abi_function(lower_seal),
            },
            Implementation {
                params: vec![
                    cipher_param(),
                    key_param(),
                    nonce_param(),
                    Parameter {
                        name: "data",
                        desc: "A string whose UTF-8 bytes are encrypted.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                    aad_param(),
                ],
                return_type: ParameterType::Named("Sealed"),
                errors: vec!["ErrInvalidArgument"],
                // The `String` overload UTF-8-encodes then re-enters the bytes
                // `AbiFunction` via the `__crypto_sealText` MFB shim — it cannot be a
                // second `AbiFunction` overload (both would collapse onto the one
                // `crypto.seal` helper symbol, whose body is bound to the first,
                // `List OF Byte`-shaped, overload).
                body: Body::Rewrite("__crypto_sealText"),
            },
        ],
    });
}
