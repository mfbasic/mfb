//! Emission helpers the per-OS `canvas::systemFontTable` backends share (plan-148).
//!
//! Every backend keeps each value it needs after an external call in a stack slot,
//! because a C call clobbers every caller-saved register (`.ai/compiler.md`). These
//! helpers are the load-from-slot / call / store-to-slot vocabulary that rule makes
//! repetitive.

use crate::codegen::engine::builder::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// A direct call to an imported C symbol (`name` without any platform prefix).
pub(crate) fn call(builder: &mut CodeBuilder, ctx: &AbiCtx, name: &str) -> Result<(), String> {
    let from = builder.current_symbol.clone();
    ctx.platform.emit_external_call(
        name,
        &from,
        ctx.platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )
}

/// C argument `arg` ← the 8 bytes at stack slot `slot`.
pub(crate) fn load_arg(builder: &mut CodeBuilder, arg: usize, slot: usize) {
    builder.emit(abi::load_u64(abi::c_arg(arg), abi::stack_pointer(), slot));
}

/// C argument `arg` ← the address of stack slot `slot`.
pub(crate) fn address_arg(builder: &mut CodeBuilder, arg: usize, slot: usize) {
    builder.emit(abi::add_immediate(
        abi::c_arg(arg),
        abi::stack_pointer(),
        slot,
    ));
}

/// Stack slot `slot` ← the C result.
pub(crate) fn store_result(builder: &mut CodeBuilder, slot: usize) {
    builder.emit(abi::store_u64(abi::c_return(0), abi::stack_pointer(), slot));
}

/// A NUL-terminated copy of `text` in a fresh stack slot; answers the slot. For the
/// handful of short, fixed C strings a backend passes (`dlsym` names, fontconfig object
/// names) — no data object to declare per target.
pub(crate) fn stack_cstring(builder: &mut CodeBuilder, name: &str, text: &str) -> usize {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    stack_bytes(builder, name, &bytes)
}

/// `bytes` in a fresh stack slot (zero-padded to a multiple of eight), written four
/// bytes at a time so every immediate is a small non-negative number on every target;
/// answers the slot. A wide string or a GUID is laid out by the caller.
pub(crate) fn stack_bytes(builder: &mut CodeBuilder, name: &str, bytes: &[u8]) -> usize {
    let mut bytes = bytes.to_vec();
    bytes.resize(bytes.len().div_ceil(8) * 8, 0);
    let slot = builder.allocate_stack_object(name, bytes.len());
    for (i, chunk) in bytes.chunks(4).enumerate() {
        let word = u32::from_le_bytes(chunk.try_into().expect("a 4-byte chunk"));
        let value = builder.temporary_vreg();
        builder.emit(abi::move_immediate(&value, "Integer", &word.to_string()));
        builder.emit(abi::store_u32(&value, abi::stack_pointer(), slot + i * 4));
    }
    slot
}

/// Allocate an MFBASIC `String` block for `capacity` bytes of text (the 8-byte length,
/// the text, and a NUL) into stack slot `result`; branch to `fail` if the arena refuses.
/// The length is left for the caller to store, and so is raising `ErrOutOfMemory` —
/// a caller holding OS objects releases them first.
pub(crate) fn alloc_string(
    builder: &mut CodeBuilder,
    capacity_slot: usize,
    result: usize,
    fail: &str,
) {
    let alloc_ok = builder.label("canvas_sysfont_string_ok");
    let size = builder.temporary_vreg();
    builder.emit(abi::load_u64(&size, abi::stack_pointer(), capacity_slot));
    builder.emit(abi::add_immediate(abi::c_arg(0), &size, 9));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.emit(abi::branch(fail));
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result,
    ));
}

/// Hand the `String` in stack slot `result` back as the call's value.
pub(crate) fn return_string(builder: &mut CodeBuilder, result: usize) {
    let answer = builder.temporary_vreg();
    builder.emit(abi::load_u64(&answer, abi::stack_pointer(), result));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &answer));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}
