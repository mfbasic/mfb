//! `crypto::hash(type, data)` — the unified `AbiFunction` hash entry point.
//!
//! Selected by a [`crypto::Hash`] enum (`SHA1`, `SHA2_224`/`SHA2_256`/`SHA2_384`/
//! `SHA2_512`), this member's `Body::abi_function` body branches on the enum ordinal
//! and dispatches to the matching always-emitted MFB software SHA `_bytes` core,
//! exactly how `func_sign`'s Ed25519 branch routes to `#crypto_ed25519Sign`. The SHA
//! math stays in MFB — this file only consolidates the per-digest `sha*` members
//! behind one `Hash`-selected surface (mirroring `generate`/`sign`/`verify` over
//! `Certificate`).
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
        origin: None,
        type_: ParameterType::list_of(ParameterType::Byte),
        location: Operand::from("void"),
        text: "crypto.hash".to_string(),
    })
}

const INTRO: &str = r#"Compute a cryptographic hash (SHA-1, SHA-2, or SHA-3) of a message, selected by a `crypto::Hash`."#;
const DESC: &str = r#"`crypto::hash(type, data)` computes the message digest of `data` for the
algorithm selected by `type` — a `crypto::Hash`: `SHA1`; the SHA-2 family
`SHA2_224`, `SHA2_256`, `SHA2_384`, `SHA2_512`; or the SHA-3 family `SHA3_224`,
`SHA3_256`, `SHA3_384`, `SHA3_512` — and returns it as a raw `List OF Byte`. The
digest length is fixed by the algorithm: 20 bytes for `SHA1`, and 28/32/48/64
bytes for the 224/256/384/512-bit widths of either family. This one call is the
package's single fixed-digest hashing surface (the variable-length SHAKE256 XOF is
`crypto::shake256`).

**`SHA1` is a legacy algorithm.** SHA-1 is not collision-resistant (practical
collisions have been public since 2017), so every source occurrence of `Hash.SHA1`
reports the non-fatal `CRYPTO_SHA1_INSECURE` warning (`2-203-0136`) — the program
still builds and runs, and the digest is the standard FIPS 180-4 value. Select it
only to interoperate with a system that requires SHA-1; use `SHA2_256` or stronger
for anything new.

The digest is a deterministic, one-way function of the message alone: the same
`type` and `data` always produce the same bytes, with no key, salt, or randomness.
Any input length is accepted, including the empty message. The function is **total**
— it never raises an error.

Two overloads accept the message. The `List OF Byte` overload hashes the raw bytes
exactly as given; the `String` overload hashes the UTF-8 encoding of the string
(equivalent to `crypto::hash(type, strings::toBytes(s))`). A digest is raw binary,
not text — stringify it with `encoding::hexEncode` or `encoding::base64Encode` to
display or store it, and compare a received digest with `crypto::constantTimeEqual`.

`hash` is a general-purpose digest and message-integrity primitive; it is **not** a
password hash. Stretch a low-entropy password into key material with the deliberately
slow, salted `crypto::pbkdf2`, and authenticate a message under a shared secret key
with `crypto::hmac` — a bare hash provides no authentication. In particular, do
**not** build a MAC as `hash(type, key ‖ message)`: `SHA1`, `SHA2_256`, and
`SHA2_512` are vulnerable to length-extension (the truncated `SHA2_224`/`SHA2_384`
resist it, and the SHA-3 sponge is immune), which lets an attacker who never saw
`key` append data and forge a valid digest. Use `crypto::hmac` for keyed
authentication.

**Implementation.** SHA-1 and SHA-2 are specified by FIPS 180-4; SHA-3 by FIPS 202
(the Keccak-f[1600] sponge at rate 1152/1088/832/576 bits with domain suffix
`0x06`). Every digest is computed in-process by a portable MFBASIC software core
over the `bits` package — no platform cryptographic library is called — so the
output is **byte-identical on macOS, Linux, and Windows** (and across
aarch64/x86-64). No loop bound, branch, or index depends on the message contents;
only the public message length does. The core is hash-generic over the `Hash`
enum, so a future `Hash` variant is supported without new code."#;
const EX: &str = r#"Hash a byte list and print it as hex:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = strings::toBytes("hello")
  LET digest AS List OF Byte = crypto::hash(Hash.SHA2_256, raw)
  io::print(encoding::hexEncode(digest))
END SUB
```

Hash a string (its UTF-8 bytes) under a different digest:

```
IMPORT crypto
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::hexEncode(crypto::hash(Hash.SHA2_512, "hello")))
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
                        ty: ParameterType::named("Hash"),
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
                        ty: ParameterType::named("Hash"),
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
