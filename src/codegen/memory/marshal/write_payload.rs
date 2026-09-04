//! The one view of a socket write payload — `tcp::write`, `udp::send` and
//! `tls::write`, on every backend — so the emitters cannot disagree about what a
//! `String` and a `List OF Byte` look like in memory (bug-497, bug-508).
//!
//! Each of those members is one overloaded name with two lowerings, selected at
//! codegen by the payload's static type (`builder_values::net_write_payload_form`).
//! The two payload blocks are laid out differently:
//!
//! | form  | length                                   | first byte                    |
//! |-------|------------------------------------------|-------------------------------|
//! | text  | `String`: `u64` at +0                     | +8                            |
//! | bytes | `List OF Byte`: `count` at +8 (header)    | `HEADER + capacity * stride`  |
//!
//! Reading one with the other's layout is not a wrong answer but an out-of-bounds
//! read whose LENGTH the payload's own bytes dictate. Before this helper each
//! backend open-coded both views: the SysV/macOS emitters were right for both
//! forms but could be handed a `String` under the byte form by a fail-open
//! selector (OS-50 — a 22-byte request read back 1024 bytes of process memory),
//! and the Schannel emitter ignored the form altogether and read every payload
//! as a `String` (bug-508 — a byte list's header word became a ~16 MiB length).
//!
//! The byte view therefore also **checks the block header** before it trusts the
//! count: a `List OF Byte` block always carries `kind = list_block_kind(Byte)`,
//! `keyType = NONE`, `valueType = BYTE` (every producer in the tree writes all
//! three — `emit_build_byte_list`, the collection-literal/slice/zip/buffer
//! emitters via `CollectionTypeLayout`, `tcp::read`, `udp::receive`, both TLS
//! reads, `fs::readBytes`, `process::receiveBytes`), while a `String` block's
//! first bytes are its length, so a mis-typed payload branches to `bad_payload`
//! (the caller raises `ErrInvalidArgument`) instead of reading a length out of
//! payload bytes. It is defence in depth behind the selector, not a substitute:
//! a `String` whose length happens to be exactly `0x0007_0002` would still pass.
//!
//! The text view is emitted instruction-for-instruction as the backends always
//! had it, so a correctly-typed `tcp::write(sock, "literal")` stays byte-identical.

use crate::codegen::collection::layout::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

/// Emit the length/source view of a write payload held in `payload` into the
/// caller's frame: the byte count to `[sp + len_slot]` (left live in `len`) and a
/// pointer to the first byte to `[sp + src_slot]` (left live in `src`).
///
/// `text` selects the `String` view; otherwise the `List OF Byte` view, whose
/// header check branches to `bad_payload` on a block that is not a byte list.
/// The caller emits that label (only when `!text` — the text view never
/// references it, and an unreferenced label would still change the text form's
/// emitted bytes) and raises `ErrInvalidArgument` there.
///
/// The three scratch operands are the byte view's; they are named for the roles
/// [`push_collection_data_base_from_capacity`] gives them, and the first doubles
/// as the header-byte scratch before the capacity load overwrites it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_write_payload_view(
    out: &mut Vec<CodeInstruction>,
    text: bool,
    payload: impl Into<Operand>,
    len: impl Into<Operand>,
    src: impl Into<Operand>,
    scratch_capacity: impl Into<Operand>,
    scratch_entry_size: impl Into<Operand>,
    scratch_product: impl Into<Operand>,
    len_slot: usize,
    src_slot: usize,
    bad_payload: &str,
) {
    let payload = payload.into();
    let len = len.into();
    let src = src.into();
    let scratch_capacity = scratch_capacity.into();
    if text {
        // String: length at +0, data at +8.
        out.extend([
            abi::load_u64(len.clone(), payload.clone(), 0),
            abi::store_u64(len, abi::stack_pointer(), len_slot),
            abi::add_immediate(src.clone(), payload, 8),
            abi::store_u64(src, abi::stack_pointer(), src_slot),
        ]);
        return;
    }
    // List OF Byte. Verify the header self-description before trusting the
    // count (bug-497): kind, keyType, valueType.
    for (offset, expected) in [
        (COLLECTION_OFFSET_KIND, byte_list_block_kind()),
        (COLLECTION_OFFSET_KEY_TYPE, COLLECTION_TYPE_NONE),
        (COLLECTION_OFFSET_VALUE_TYPE, COLLECTION_TYPE_BYTE),
    ] {
        out.extend([
            abi::load_u8(scratch_capacity.clone(), payload.clone(), offset),
            abi::compare_immediate(scratch_capacity.clone(), expected.to_string().as_str()),
            abi::branch_ne(bad_payload),
        ]);
    }
    // Bytes live inline in the data region, which begins past the
    // CAPACITY-sized entry array (an append-built list carries spare capacity,
    // so a COUNT-based base mis-addresses it — bug-157).
    out.extend([
        abi::load_u64(len.clone(), payload.clone(), COLLECTION_OFFSET_COUNT),
        abi::store_u64(len, abi::stack_pointer(), len_slot),
    ]);
    push_collection_data_base_from_capacity(
        out,
        src.clone(),
        payload,
        scratch_capacity,
        scratch_entry_size,
        scratch_product,
    );
    out.push(abi::store_u64(src, abi::stack_pointer(), src_slot));
}
