//! Shared clean-room codegen seam for the `SymmetricCipher`-typed `AbiFunction`
//! AEAD members (`crypto::seal` / `crypto::open`).
//!
//! `crypto::seal(cipher, key, nonce, data[, aad])` and
//! `crypto::open(cipher, key, nonce, ciphertext, tag[, aad])` select an AEAD cipher by
//! the `SymmetricCipher` enum ordinal and branch-link to the always-emitted MFB software
//! AEAD core (`__crypto_aes256GcmSeal`/`__crypto_chacha20Poly1305Seal` for seal,
//! `__crypto_aes256GcmOpen`/`__crypto_chacha20Poly1305Open` for open). Like
//! [`super::gen_hash`] (and unlike [`super::gen_cert`]), this seam emits **no** data
//! objects and needs **no** platform imports — every arm just routes the operation's
//! argument operands into the core's argument registers and calls it, exactly how
//! `func_sign`'s Ed25519 branch calls `#crypto_ed25519Sign`. The AEAD math stays in MFB.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

// ---------------------------------------------------------------------------
// SymmetricCipher ordinals (must match the `SymmetricCipher` enum declaration
// order in `crypto/mod.rs`).
// ---------------------------------------------------------------------------
// AES-256-GCM is the fall-through ordinal (never compared), but named for the contract.
#[allow(dead_code)]
pub(crate) const ORD_AES256GCM: &str = "0";
pub(crate) const ORD_CHACHA20POLY1305: &str = "1";

/// The AEAD operation this dispatch routes to — picks the `Seal` vs `Open` MFB core.
#[derive(Clone, Copy)]
pub(crate) enum Op {
    Seal,
    Open,
}

impl Op {
    fn suffix(self) -> &'static str {
        match self {
            Op::Seal => "Seal",
            Op::Open => "Open",
        }
    }
}

/// Emit the `SymmetricCipher`-ordinal dispatch for `crypto::seal`/`crypto::open`: branch
/// on `ord` to the AEAD core matching the cipher, routing `arg_ops` into the core's
/// argument registers (0..). For seal `arg_ops` is `[key, nonce, plaintext, aad]`; for
/// open `[key, nonce, ciphertext, tag, aad]`. Every arm leaves the result
/// (`Sealed` for seal, plaintext `List OF Byte` for open) in the result registers and
/// branches to `done`; the caller emits the shared `done`/return.
pub(crate) fn emit_dispatch(
    builder: &mut CodeBuilder,
    symbol: &str,
    ord: &Operand,
    arg_ops: &[Operand],
    op: Op,
    ctx: &AbiCtx,
    done: &str,
) -> Result<(), String> {
    let chacha = format!("{symbol}_chacha");
    builder.instructions.extend([
        abi::compare_immediate(ord, ORD_CHACHA20POLY1305),
        abi::branch_eq(&chacha),
    ]);
    // AES-256-GCM (ordinal 0) falls through here.
    emit_core(builder, "aes256Gcm", op, arg_ops, ctx, done)?;
    builder.instructions.push(abi::label(&chacha));
    emit_core(builder, "chacha20Poly1305", op, arg_ops, ctx, done)?;
    Ok(())
}

/// Route `arg_ops` into the AEAD core's argument registers (0..) and call the
/// always-emitted `#crypto_<cipher><Seal|Open>` MFB software core, wrapping the internal
/// call in the Win64 shadow-space bracket. The core leaves its result in the result
/// registers, so branch to the shared `done`.
///
/// The operands arrive one argument register higher than the core wants (the leading
/// `SymmetricCipher` ordinal occupies argument register 0), so this is a downward shift:
/// each `argument_register(i)` is written from `arg_ops[i]` (which reads
/// `argument_register(i + 1)`) in increasing `i`, so no not-yet-read source is clobbered.
fn emit_core(
    builder: &mut CodeBuilder,
    cipher: &str,
    op: Op,
    arg_ops: &[Operand],
    ctx: &AbiCtx,
    done: &str,
) -> Result<(), String> {
    for (i, arg) in arg_ops.iter().enumerate() {
        builder
            .instructions
            .push(abi::move_register(abi::argument_register(i)?, arg.clone()));
    }
    let core =
        crate::target::shared::nir::function_symbol(&format!("#crypto_{cipher}{}", op.suffix()));
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
