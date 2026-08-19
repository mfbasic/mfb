//! `crypto::open(cipher, key, nonce, ciphertext, tag[, aad])` — the unified `AbiFunction`
//! AEAD opening entry point.
//!
//! Selected by a [`crypto::SymmetricCipher`] enum (`AES256GCM`/`CHACHA20POLY1305`), this
//! member's `Body::abi_function` body branches on the enum ordinal and dispatches to the
//! matching always-emitted MFB software AEAD `Open` core, exactly how `func_sign`'s
//! Ed25519 branch routes to `#crypto_ed25519Sign`. The AEAD math (constant-time tag
//! verification, failing closed with `ErrAuthenticationFailed`) stays in MFB — this file
//! only consolidates the two `*Open` members behind one `SymmetricCipher`-selected surface
//! (mirroring `hash` over `Hash`).
//!
//! Two overloads mirror the per-cipher `*Open` members. The five-argument
//! (`ciphertext`, `tag`) form is the `AbiFunction` proper: it branches on the ordinal and
//! calls the `Open` cores. The `crypto::Sealed` form is a `Body::Rewrite` to the
//! `__crypto_openSealed` MFB shim (registered by [`super::helper_open_sealed`]) which
//! unpacks the record's `ciphertext`/`tag` fields and re-enters the five-argument `open` —
//! it cannot be a second `AbiFunction` overload (both would collapse onto the one
//! `crypto.open` helper symbol, whose body is bound to the first overload). The shared
//! per-ordinal dispatch scaffolding lives in [`super::gen_cipher`].
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

/// The `AbiFunction` body for the five-argument overload of `crypto::open`. `args[0]` is
/// the `SymmetricCipher` ordinal; `args[1..=5]` are the `key`, `nonce`, `ciphertext`,
/// `tag`, and `aad` collection pointers in their argument registers. Branches on the
/// ordinal to the matching MFB software AEAD `Open` core, which verifies the tag in
/// constant time and either leaves the plaintext `List OF Byte` in the result registers or
/// raises `ErrAuthenticationFailed` — so this is a self-managed fallible ABI body that
/// returns the `void` sentinel (the wrapper adds no epilogue). The `crypto::Sealed`
/// overload reaches the equivalent path through the `__crypto_openSealed` shim.
pub(crate) fn lower_open(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ord = args[0].location.clone();
    // Core takes (key, nonce, ciphertext, tag, aad).
    let arg_ops = [
        args[1].location.clone(),
        args[2].location.clone(),
        args[3].location.clone(),
        args[4].location.clone(),
        args[5].location.clone(),
    ];

    let done = format!("{symbol}_done");
    gen_cipher::emit_dispatch(
        builder,
        &symbol,
        &ord,
        &arg_ops,
        gen_cipher::Op::Open,
        ctx,
        &done,
    )?;

    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);

    Ok(ValueResult {
        type_: "List OF Byte".to_string(),
        location: Operand::from("void"),
        text: "crypto.open".to_string(),
    })
}

const INTRO: &str =
    r#"Verify and decrypt an AEAD-sealed message, selected by a `crypto::SymmetricCipher`."#;
const DESC: &str = r#"`crypto::open(cipher, key, nonce, ciphertext, tag)` verifies the authentication
`tag` and decrypts `ciphertext` with the authenticated cipher selected by `cipher`
(a `crypto::SymmetricCipher`: `AES256GCM` or `CHACHA20POLY1305`), returning the
recovered plaintext as a `List OF Byte`. It is the unified front door for the two
symmetric AEAD ciphers behind one `SymmetricCipher`-selected call, the inverse of
`crypto::seal(cipher, …)`.

The tag is verified in **constant time** and the function **fails closed**: any
mismatch — a corrupted ciphertext, a wrong tag, a wrong key or nonce, or a
different `aad` than was sealed — raises `ErrAuthenticationFailed` and returns no
plaintext. The same `key`, `nonce`, and `aad` supplied to `crypto::seal` must be
supplied here; `aad` defaults to the empty list when omitted.

A `crypto::Sealed` overload takes the record returned by `crypto::seal` directly
(`crypto::open(cipher, key, nonce, sealed)`), unpacking its `ciphertext` and `tag`
fields for you. The ciphers are portable software cores computed over the `bits`
package, so decryption is **byte-identical on every target** (macOS/Linux/Windows,
aarch64/x86-64) and uses no platform crypto library."#;
const EX: &str = r#"Seal, then open the `Sealed` record directly:

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

Open the ciphertext and tag explicitly, with additional authenticated data:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET secret AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
  LET header AS List OF Byte = strings::toBytes("v1;msg-42")
  LET box AS crypto::Sealed = crypto::seal(SymmetricCipher.CHACHA20POLY1305, secret, nonce, "attack at dawn", header)
  LET clear AS List OF Byte = crypto::open(SymmetricCipher.CHACHA20POLY1305, secret, nonce, box.ciphertext, box.tag, header)
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
        desc: "The 12-byte nonce; must match the nonce used to seal.",
        aliases: &[],
        ty: bytes(),
        default: DefaultValue::None,
    };
    let aad_param = || Parameter {
        name: "aad",
        desc:
            "Optional additional authenticated data; must match the seal `aad`. Defaults to empty.",
        aliases: &[],
        ty: bytes(),
        default: DefaultValue::Fill {
            type_name: bytes(),
            expr: "",
        },
    };
    pkg.add_function(RegistryFunction {
        name: "open",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some(
            "crypto::SymmetricCipher, List OF Byte, List OF Byte, (List OF Byte, List OF Byte or crypto::Sealed)[, List OF Byte]",
        ),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    cipher_param(),
                    key_param(),
                    nonce_param(),
                    Parameter {
                        name: "ciphertext",
                        desc: "The ciphertext bytes to decrypt (the `ciphertext` field of a `crypto::Sealed`).",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "tag",
                        desc: "The authentication tag (the `tag` field of a `crypto::Sealed`).",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    aad_param(),
                ],
                return_type: bytes(),
                errors: vec!["ErrAuthenticationFailed", "ErrInvalidArgument"],
                body: Body::abi_function(lower_open),
            },
            Implementation {
                params: vec![
                    cipher_param(),
                    key_param(),
                    nonce_param(),
                    Parameter {
                        name: "sealed",
                        desc: "The `crypto::Sealed` record returned by `crypto::seal`.",
                        aliases: &[],
                        ty: ParameterType::Named("Sealed"),
                        default: DefaultValue::None,
                    },
                    aad_param(),
                ],
                return_type: bytes(),
                errors: vec!["ErrAuthenticationFailed", "ErrInvalidArgument"],
                // The `Sealed` overload unpacks `.ciphertext`/`.tag` then re-enters the
                // five-argument `AbiFunction` via the `__crypto_openSealed` MFB shim — it
                // cannot be a second `AbiFunction` overload (both would collapse onto the
                // one `crypto.open` helper symbol, whose body is bound to the first
                // overload).
                body: Body::Rewrite("__crypto_openSealed"),
            },
        ],
    });
}
