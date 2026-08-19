//! Shared clean-room codegen seam for the `Hash`-typed `AbiFunction` crypto members.
//!
//! `crypto::hash(Hash, data)` selects a SHA-2 digest by the `Hash` enum ordinal and
//! branch-links to the always-emitted MFB software SHA core
//! (`__crypto_sha{224,256,384,512}_{bytes,text}`). Unlike [`super::gen_cert`] (which
//! reproduces the platform key sequences and owns data objects), this seam emits **no**
//! data objects and needs **no** platform imports — every arm just routes the single
//! `data` argument into the core's first argument register and calls it, exactly how
//! `func_sign`'s Ed25519 branch calls `#crypto_ed25519Sign`. The SHA math stays in MFB.
//!
//! It is shared by both `hash` overloads (the `List OF Byte` `_bytes` core family and
//! the `String` `_text` core family, selected by `is_text`), and is the consolidation
//! point for any future `hmac(Hash,…)`/`hkdf(Hash,…)`/`pbkdf2(Hash,…)` members.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

// ---------------------------------------------------------------------------
// Hash ordinals (must match the `Hash` enum declaration order in `crypto/mod.rs`).
// ---------------------------------------------------------------------------
// SHA-224 is the fall-through ordinal (never compared), but named for the contract.
#[allow(dead_code)]
pub(crate) const ORD_SHA224: &str = "0";
pub(crate) const ORD_SHA256: &str = "1";
pub(crate) const ORD_SHA384: &str = "2";
pub(crate) const ORD_SHA512: &str = "3";

/// Emit the `Hash`-ordinal dispatch for `crypto::hash`: branch on `ord` to the SHA-2
/// core matching the digest, routing the single `data` operand into the core's first
/// argument register. `is_text` picks the `_text` (String) vs `_bytes` (List OF Byte)
/// core family. Every arm leaves the digest `List OF Byte` in the result registers and
/// branches to `done`; the caller emits the shared `done`/return.
pub(crate) fn emit_dispatch(
    builder: &mut CodeBuilder,
    symbol: &str,
    ord: &Operand,
    data_op: Operand,
    is_text: bool,
    ctx: &AbiCtx,
    done: &str,
) -> Result<(), String> {
    let suffix = if is_text { "text" } else { "bytes" };
    let sha256 = format!("{symbol}_{suffix}_sha256");
    let sha384 = format!("{symbol}_{suffix}_sha384");
    let sha512 = format!("{symbol}_{suffix}_sha512");
    builder.instructions.extend([
        abi::compare_immediate(ord, ORD_SHA256),
        abi::branch_eq(&sha256),
        abi::compare_immediate(ord, ORD_SHA384),
        abi::branch_eq(&sha384),
        abi::compare_immediate(ord, ORD_SHA512),
        abi::branch_eq(&sha512),
    ]);
    // SHA-224 (ordinal 0) falls through here.
    emit_core(builder, "224", suffix, data_op.clone(), ctx, done)?;
    builder.instructions.push(abi::label(&sha256));
    emit_core(builder, "256", suffix, data_op.clone(), ctx, done)?;
    builder.instructions.push(abi::label(&sha384));
    emit_core(builder, "384", suffix, data_op.clone(), ctx, done)?;
    builder.instructions.push(abi::label(&sha512));
    emit_core(builder, "512", suffix, data_op, ctx, done)?;
    Ok(())
}

/// Route `data_op` into the core's first argument register and call the always-emitted
/// `#crypto_sha<n>_<bytes|text>` MFB software core, wrapping the internal call in the
/// Win64 shadow-space bracket. The core leaves the digest `List OF Byte` in the result
/// registers, so branch to the shared `done`.
fn emit_core(
    builder: &mut CodeBuilder,
    n: &str,
    suffix: &str,
    data_op: Operand,
    ctx: &AbiCtx,
    done: &str,
) -> Result<(), String> {
    builder
        .instructions
        .push(abi::move_register(abi::argument_register(0)?, data_op));
    let core = crate::target::shared::nir::function_symbol(&format!("#crypto_sha{n}_{suffix}"));
    // Win64 mandates 32 bytes of caller-reserved shadow space around every call.
    let win64 = matches!(ctx.platform.family(), PlatformFamily::Windows);
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&core);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(done));
    Ok(())
}
