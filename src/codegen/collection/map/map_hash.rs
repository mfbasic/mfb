//! bug-770: the home slot of a `Map`/`Set` key in the hash index — the ONE
//! definition every index site emits.
//!
//! The index is only correct while every writer and reader computes the same
//! home slot for a key: the inline probe (`CodeBuilder::emit_map_probe`), the
//! runtime probe, the lazy rebuild, the incremental insert
//! (`_mfb_rt_map_probe`/`_mfb_rt_map_build_buckets`/`_mfb_rt_map_bucket_put`),
//! and the backward-shift delete and its renumbering
//! (`map_bucket_remove::home_slot`). A site that disagrees leaves a present key
//! silently unreachable, so none of them spells the hash out; they all splice
//! these fragments in.
//!
//! The hash is a pure function of the key's stored BYTES, dispatched on their
//! length — never on the static key type, which the shared runtime helpers do
//! not know:
//!
//! * **8 bytes** (`Integer`, `Float`, `Fixed`, and a `String` that happens to be
//!   8 bytes long): the bytes read as one little-endian word `k`, then
//!   `slot = umulh(k * MAP_KEY_WORD_MULTIPLIER, bucketCount)` — a Fibonacci
//!   (multiplicative) hash whose high bits are range-reduced onto the
//!   `2 * capacity` slots with a multiply-high, so there is no division. An
//!   arithmetic progression of keys (`i`, `2i`, `1024i`) lands on an evenly
//!   spread Weyl sequence of slots rather than one residue class.
//! * **any other length** (a 1-byte `Byte`/`Boolean`/`Nothing`, a `String`):
//!   FNV-1a over the bytes, `mod bucketCount`.
//!
//! A word is loaded with one 8-byte load only when its address is 8-aligned; a
//! fixed-width key always is (its payload alignment is 8), but a `String` key's
//! bytes need not be, and riscv64 may service an unaligned load by a kernel
//! trap, so an unaligned word is assembled from its 8 bytes instead.

use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

/// The length, in bytes, of a key that takes the word hash.
pub(crate) const MAP_WORD_KEY_LENGTH: &str = "8";

/// The registers [`home_slot_of_key_bytes`] reads, writes and clobbers.
#[derive(Clone, Copy)]
pub(crate) struct KeyHashRegs {
    /// The key's first byte (preserved).
    pub(crate) key_ptr: VirtualRegister,
    /// The key's byte length (preserved).
    pub(crate) key_len: VirtualRegister,
    /// `2 * capacity` (preserved).
    pub(crate) bucket_count: VirtualRegister,
    /// Out: the home slot, in `[0, bucketCount)`.
    pub(crate) slot: VirtualRegister,
    /// Scratch (clobbered).
    pub(crate) hash: VirtualRegister,
    /// Scratch (clobbered).
    pub(crate) byte: VirtualRegister,
    /// Scratch (clobbered).
    pub(crate) cursor: VirtualRegister,
    /// Scratch (clobbered).
    pub(crate) remaining: VirtualRegister,
}

/// `slot = umulh(word * MAP_KEY_WORD_MULTIPLIER, bucketCount)`: the home slot of
/// an 8-byte key already loaded as `word`. Clobbers `word`.
pub(crate) fn word_home_slot(
    word: VirtualRegister,
    bucket_count: VirtualRegister,
    slot: VirtualRegister,
) -> Vec<CodeInstruction> {
    vec![
        abi::move_immediate(slot, "Integer", MAP_KEY_WORD_MULTIPLIER),
        abi::multiply_registers(word, word, slot),
        abi::unsigned_multiply_high_registers(slot, word, bucket_count),
    ]
}

/// `dst` = the 8 bytes at `ptr` as a little-endian word: one load when `ptr` is
/// 8-aligned, else assembled a byte at a time (see the module doc). Clobbers
/// `byte` and `cursor`; `ptr` is preserved.
pub(crate) fn load_key_word(
    labels: &str,
    dst: VirtualRegister,
    ptr: VirtualRegister,
    byte: VirtualRegister,
    cursor: VirtualRegister,
) -> Vec<CodeInstruction> {
    let unaligned = format!("{labels}_wunaligned");
    let assemble = format!("{labels}_wbyte");
    let loaded = format!("{labels}_wloaded");
    vec![
        abi::move_immediate(byte, "Integer", "7"),
        abi::and_registers(byte, ptr, byte),
        abi::compare_immediate(byte, "0"),
        abi::branch_ne(&unaligned),
        abi::load_u64(dst, ptr, 0),
        abi::branch(&loaded),
        // Little-endian: the last byte is the most significant, so walk down.
        abi::label(&unaligned),
        abi::move_immediate(dst, "Integer", "0"),
        abi::add_immediate(cursor, ptr, 8),
        abi::label(&assemble),
        abi::subtract_immediate(cursor, cursor, 1),
        abi::load_u8(byte, cursor, 0),
        abi::shift_left_immediate(dst, dst, 8),
        abi::or_registers(dst, dst, byte),
        abi::compare_registers(cursor, ptr),
        abi::branch_ne(&assemble),
        abi::label(&loaded),
    ]
}

/// `slot = FNV-1a(key bytes) mod bucketCount` — the hash of every key whose
/// length is not [`MAP_WORD_KEY_LENGTH`]. Clobbers `hash`, `byte`, `cursor`,
/// `remaining`; `slot` holds the FNV prime until the final reduction.
pub(crate) fn fnv_home_slot(labels: &str, r: &KeyHashRegs) -> Vec<CodeInstruction> {
    let hloop = format!("{labels}_hloop");
    let hdone = format!("{labels}_hdone");
    vec![
        abi::move_immediate(r.slot, "Integer", FNV1A_PRIME),
        abi::move_immediate(r.hash, "Integer", FNV1A_BASIS),
        abi::move_register(r.cursor, r.key_ptr),
        abi::move_register(r.remaining, r.key_len),
        abi::label(&hloop),
        abi::compare_immediate(r.remaining, "0"),
        abi::branch_eq(&hdone),
        abi::load_u8(r.byte, r.cursor, 0),
        abi::exclusive_or_registers(r.hash, r.hash, r.byte),
        abi::multiply_registers(r.hash, r.hash, r.slot),
        abi::add_immediate(r.cursor, r.cursor, 1),
        abi::subtract_immediate(r.remaining, r.remaining, 1),
        abi::branch(&hloop),
        abi::label(&hdone),
        abi::unsigned_divide_registers(r.slot, r.hash, r.bucket_count),
        abi::multiply_subtract_registers(r.slot, r.slot, r.bucket_count, r.hash),
    ]
}

/// The home slot of the key whose bytes are `key_ptr`/`key_len`, dispatched on
/// the length: the word hash for 8 bytes, FNV-1a otherwise. The one fragment the
/// runtime helpers splice in; the inline probe, which knows the key type, emits
/// the same two arms directly ([`word_home_slot`], [`fnv_home_slot`]) and routes
/// the one key it cannot classify statically (a `String`) through here.
pub(crate) fn home_slot_of_key_bytes(labels: &str, r: &KeyHashRegs) -> Vec<CodeInstruction> {
    let bytes = format!("{labels}_fnv");
    let done = format!("{labels}_hashed");
    let mut out = vec![
        abi::compare_immediate(r.key_len, MAP_WORD_KEY_LENGTH),
        abi::branch_ne(&bytes),
    ];
    out.extend(load_key_word(labels, r.hash, r.key_ptr, r.byte, r.cursor));
    out.extend(word_home_slot(r.hash, r.bucket_count, r.slot));
    out.push(abi::branch(&done));
    out.push(abi::label(&bytes));
    out.extend(fnv_home_slot(labels, r));
    out.push(abi::label(&done));
    out
}
