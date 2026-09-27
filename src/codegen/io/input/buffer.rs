//! The input buffer (bug-699 Fix Design layer 1): per stdin subscriber, the
//! bytes pulled from the source but not yet consumed by any reader, oldest first.
//!
//! It sits between the byte source — the stdin broadcast log in a console build,
//! the window input pipe in an `--app` build, both behind the plan-94-B mouse
//! decoder — and the readers:
//!
//! ```text
//! source (log / pipe) -> mouse decoder -> INPUT BUFFER -> readers
//! ```
//!
//! A reader that must look ahead to decide what it has (the decoder: is this ESC
//! the Escape key or the start of a sequence? is a combining mark coming?) pulls
//! bytes INTO the buffer and consumes only its own token's bytes out of the
//! front. Every reader takes buffered bytes before it reads new ones, so a byte
//! one reader looked at and left is the next byte another reader gets — mixing
//! readers never loses or reorders input.
//!
//! The region lives in arena state ([`super::INPUT_STATE_LEN_OFFSET`],
//! [`super::INPUT_STATE_BUF_OFFSET`]), so each thread's buffer is its own and no
//! lock is needed — the same reason the mouse decoder's buffer is there. It is
//! reserved only in a program that has a reader which can leave bytes in it
//! ([`super::program_uses_input_buffer`]); in any other program none of this is
//! emitted and the read paths are exactly what they were.

use super::{INPUT_STATE_BUF_OFFSET, INPUT_STATE_LEN_OFFSET, INPUT_STATE_PENDING_OFFSET};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::ARENA_STATE_REGISTER;
use crate::codegen::io::stdin::field_addr;
use crate::target::shared::abi;

/// `dst = &region` for the calling thread. Computed in a register rather than
/// used as a load displacement: arena-state offsets sit past rv64's 12-bit
/// immediate (the stdin fields do the same, `stdin_broadcast::field_addr`).
fn emit_region_address(dst: impl Into<abi::Reg>, state_offset: usize, ctx: &mut EmitCtx) {
    let dst: abi::Reg = dst.into();
    field_addr(dst, ARENA_STATE_REGISTER, state_offset, ctx.instructions);
}

/// Branch to `buffered` when the calling thread's input buffer holds a byte.
///
/// `io::pollInput` asks this first: a buffered byte is input a read returns at
/// once, and it is invisible to both the broadcast log and `poll(fd 0)`.
pub(crate) fn emit_branch_if_buffered(
    buffered: &str,
    state_offset: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) {
    let region = vregs.next();
    let len = vregs.next();
    emit_region_address(&region, state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u64(&len, &region, INPUT_STATE_LEN_OFFSET),
        abi::compare_immediate(&len, "0"),
        abi::branch_ne(buffered),
    ]);
}

/// Drop the oldest `count` bytes from the buffer (`count` <= the length): shift
/// the rest to the front and shorten it. The buffer is small and a token is a
/// handful of bytes, so a copy is cheaper to get right than a ring's wrap. The
/// front changes, so nothing is waiting to be classified any more: the pending
/// timestamp is cleared.
pub(crate) fn emit_consume(
    count: impl Into<abi::Reg>,
    state_offset: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) {
    let count: abi::Reg = count.into();
    let symbol = ctx.symbol;
    let tag = ctx.instructions.len();
    let copy = format!("{symbol}_input_consume_copy_{tag}");
    let done = format!("{symbol}_input_consume_done_{tag}");
    let region = vregs.next();
    let len = vregs.next();
    let from = vregs.next();
    let to = vregs.next();
    let end = vregs.next();
    let byte = vregs.next();
    emit_region_address(&region, state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u64(&len, &region, INPUT_STATE_LEN_OFFSET),
        abi::add_immediate(&to, &region, INPUT_STATE_BUF_OFFSET),
        abi::add_registers(&from, &to, count),
        abi::add_registers(&end, &to, &len),
        abi::subtract_registers(&len, &len, count),
        abi::store_u64(&len, &region, INPUT_STATE_LEN_OFFSET),
        abi::store_u64(abi::ZERO, &region, INPUT_STATE_PENDING_OFFSET),
        abi::label(&copy),
        // while from < end: *to++ = *from++
        abi::compare_registers(&end, &from),
        abi::branch_ls(&done),
        abi::load_u8(&byte, &from, 0),
        abi::store_u8(&byte, &to, 0),
        abi::add_immediate(&from, &from, 1),
        abi::add_immediate(&to, &to, 1),
        abi::branch(&copy),
        abi::label(&done),
    ]);
}

/// A plain reader's front on the buffer: when it holds a byte, take the oldest
/// into `sp + byte_offset` and go to `got` with the result register and flags
/// exactly as a one-byte read leaves them (`1`, compared with 0); when it is
/// empty, fall through to the source read.
pub(crate) fn emit_take_buffered_byte(
    got: &str,
    state_offset: usize,
    byte_offset: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) {
    let symbol = ctx.symbol;
    let empty = format!("{symbol}_input_empty_{}", ctx.instructions.len());
    let region = vregs.next();
    let len = vregs.next();
    let byte = vregs.next();
    let one = vregs.next();
    emit_region_address(&region, state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u64(&len, &region, INPUT_STATE_LEN_OFFSET),
        abi::compare_immediate(&len, "0"),
        abi::branch_eq(&empty),
        abi::load_u8(&byte, &region, INPUT_STATE_BUF_OFFSET),
        abi::store_u8(&byte, abi::stack_pointer(), byte_offset),
        abi::move_immediate(&one, "Integer", "1"),
    ]);
    emit_consume(&one, state_offset, ctx, vregs);
    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch(got),
        abi::label(&empty),
    ]);
}

/// Give `count` bytes at `src` back to the FRONT of the buffer, ahead of
/// whatever it holds — the inverse of [`emit_consume`]. How a timed line read
/// leaves its half-read line for the next read, and how a scalar read keeps the
/// rest of a key that typed several scalars. Branches to `full` (and changes
/// nothing) when the buffer cannot hold them. `src` must not point into the
/// buffer itself.
pub(crate) fn emit_unread(
    symbol: &str,
    src: impl Into<abi::Reg>,
    count: impl Into<abi::Reg>,
    state_offset: usize,
    full: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let src: abi::Reg = src.into();
    let count: abi::Reg = count.into();
    let tag = instructions.len();
    let shift = format!("{symbol}_input_unread_shift_{tag}");
    let copy = format!("{symbol}_input_unread_copy_{tag}");
    let done = format!("{symbol}_input_unread_done_{tag}");
    let region = vregs.next();
    let len = vregs.next();
    let total = vregs.next();
    let cap = vregs.next();
    let from = vregs.next();
    let to = vregs.next();
    let base = vregs.next();
    let byte = vregs.next();
    let i = vregs.next();
    field_addr(region, ARENA_STATE_REGISTER, state_offset, instructions);
    instructions.extend([
        abi::load_u64(&len, &region, INPUT_STATE_LEN_OFFSET),
        abi::add_registers(&total, &len, count),
        abi::move_immediate(&cap, "Integer", &super::INPUT_BUFFER_CAPACITY.to_string()),
        abi::compare_registers(&total, &cap),
        abi::branch_hi(full),
        // Move the buffered bytes up by `count`, last byte first.
        abi::add_immediate(&base, &region, INPUT_STATE_BUF_OFFSET),
        abi::add_registers(&from, &base, &len),
        abi::add_registers(&to, &from, count),
        abi::label(&shift),
        abi::compare_registers(&from, &base),
        abi::branch_ls(&copy),
        abi::subtract_immediate(&from, &from, 1),
        abi::subtract_immediate(&to, &to, 1),
        abi::load_u8(&byte, &from, 0),
        abi::store_u8(&byte, &to, 0),
        abi::branch(&shift),
        // Then the given bytes into the front.
        abi::label(&copy),
        abi::move_immediate(&i, "Integer", "0"),
        abi::label(&format!("{copy}_loop")),
        abi::compare_registers(&i, count),
        abi::branch_eq(&done),
        abi::add_registers(&from, src, &i),
        abi::load_u8(&byte, &from, 0),
        abi::add_registers(&to, &base, &i),
        abi::store_u8(&byte, &to, 0),
        abi::add_immediate(&i, &i, 1),
        abi::branch(&format!("{copy}_loop")),
        abi::label(&done),
        abi::store_u64(&total, &region, INPUT_STATE_LEN_OFFSET),
        abi::store_u64(abi::ZERO, &region, INPUT_STATE_PENDING_OFFSET),
    ]);
}
