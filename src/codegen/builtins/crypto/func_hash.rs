//! `crypto::hash(type, data)` — the unified `AbiFunction` hash entry point.
//!
//! Selected by a [`crypto::Hash`] enum (`SHA224`/`SHA256`/`SHA384`/`SHA512`), this
//! member's `Body::abi_function` body branches on the enum ordinal and dispatches to
//! the matching always-emitted MFB software SHA `_bytes` core, exactly how `func_sign`'s
//! Ed25519 branch routes to `#crypto_ed25519Sign`. The SHA math stays in MFB — this file
//! only consolidates the four `sha*` members behind one `Hash`-selected surface
//! (mirroring `generate`/`sign`/`verify` over `Certificate`).
//!
//! Two overloads mirror the per-digest `sha*` members. The `List OF Byte` form is the
//! `AbiFunction` proper: it branches on the ordinal and calls the `_bytes` cores. The
//! `String` form is a `Body::Rewrite` to the `__crypto_hashText` MFB shim (registered by
//! [`super::helper_hash_text`]) which UTF-8-encodes the string and re-enters the bytes
//! `AbiFunction` — so a `String` argument reaches the `_text`-equivalent path
//! (`strings::toBytes` then the same SHA core) and the two overloads never share the one
//! `crypto.hash` runtime symbol (an `AbiFunction` member emits exactly one helper, and
//! `abi_function_lower` binds the first overload's body; a second `AbiFunction` overload
//! would silently reuse that same `List OF Byte`-shaped body on a `String` pointer). The
//! shared per-ordinal dispatch scaffolding lives in [`super::gen_hash`].

use super::gen_hash;
use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// The `AbiFunction` body for the `List OF Byte` overload of `crypto::hash`. `args[0]`
/// is the `Hash` ordinal, `args[1]` the data collection pointer in its argument
/// register. Branches on the ordinal to the matching MFB software SHA `_bytes` core,
/// which leaves the digest `List OF Byte` in the result registers — so this is a
/// self-managed fallible ABI body that returns the `void` sentinel (the wrapper adds
/// no epilogue). The `String` overload reaches the equivalent path through the
/// `__crypto_hashText` shim, so this body only ever runs over a `List OF Byte`.
pub(crate) fn lower_hash(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ord = args[0].location.clone();
    let data_op = args[1].location.clone();

    let done = format!("{symbol}_done");
    gen_hash::emit_dispatch(builder, &symbol, &ord, data_op, false, ctx, &done)?;

    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);

    Ok(ValueResult {
        type_: "List OF Byte".to_string(),
        location: Operand::from("void"),
        text: "crypto.hash".to_string(),
    })
}

const INTRO: &str =
    r#"Compute a SHA-2 cryptographic hash of a message, selected by a `crypto::Hash`."#;
const DESC: &str = r#"`crypto::hash(type, data)` computes the SHA-2 message digest of `data` for the
algorithm selected by `type` (a `crypto::Hash`: `SHA224`, `SHA256`, `SHA384`, or
`SHA512`), and returns it as a raw `List OF Byte` — 28, 32, 48, or 64 bytes
respectively. It is the unified front door for the four SHA-2 hashes
(`SHA224`/`SHA256`/`SHA384`/`SHA512`) behind one `Hash`-selected call.

The digest is a deterministic function of the input alone: the same message and
algorithm always produce the same bytes, with no keying, salting, or randomness.
The function is **total** — every input, including the empty message, yields a
digest and it never raises an error.

The hashes are portable software cores computed over the `bits` package, so their
output is **byte-identical on every target** (macOS/Linux/Windows, aarch64/x86-64)
and use no platform crypto library. A digest is raw binary, not text; stringify it
with `encoding::hexEncode` or `encoding::base64Encode` to display or store it.

`hash` is a general-purpose digest and message-integrity primitive. It is **not** a
password hash on its own; derive password material with `crypto::pbkdf2Sha256`, and
authenticate messages under a shared key with `crypto::hmacSha256`. The
`List OF Byte` overload hashes the raw bytes as given; the `String` overload hashes
the string's UTF-8 encoding."#;
const EX: &str = r#"Hash a byte list and print it as hex:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = strings::toBytes("hello")
  LET digest AS List OF Byte = crypto::hash(Hash.SHA256, raw)
  io::print(encoding::hexEncode(digest))
END SUB
```

Hash a string (its UTF-8 bytes) under a different digest:

```
IMPORT crypto
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::hexEncode(crypto::hash(Hash.SHA512, "hello")))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hash",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("crypto::Hash, List OF Byte or String"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "type",
                        desc: "The hash algorithm to compute.",
                        aliases: &[],
                        ty: ParameterType::Named("Hash"),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "data",
                        desc:
                            "The bytes to hash. Any length is accepted, including the empty list.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                ],
                return_type: bytes(),
                errors: vec![],
                body: Body::abi_function(lower_hash),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "type",
                        desc: "The hash algorithm to compute.",
                        aliases: &[],
                        ty: ParameterType::Named("Hash"),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "data",
                        desc: "A string whose UTF-8 bytes are hashed.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                ],
                return_type: bytes(),
                errors: vec![],
                // The `String` overload UTF-8-encodes then re-enters the bytes
                // `AbiFunction` via the `__crypto_hashText` MFB shim — it cannot be a
                // second `AbiFunction` overload (both would collapse onto the one
                // `crypto.hash` helper symbol, whose body is bound to the first,
                // `List OF Byte`-shaped, overload).
                body: Body::Rewrite("__crypto_hashText"),
            },
        ],
    });
}
