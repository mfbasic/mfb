//! Generic native-codegen emitters shared by the `dlopen`-based runtime-helper
//! backends (`tls`, `crypto`, `crypto_ec`, `audio`). None of these routines has
//! any package affinity: they marshal `List OF Byte` values, load data-symbol
//! addresses, return blocks to the arena, wipe key-material scratch, and build
//! the standard `Result` error tail — the platform-neutral scaffolding every
//! backend needs.
//!
//! They lived in `tls/mod.rs` for the accidental reason that `tls` was the first
//! `dlopen` package (bug-330). That made `tls` the de-facto home for compiler
//! emitters that have nothing to do with transport-layer security, so a new
//! backend author had no discoverable place to reuse them and wrote a local
//! copy instead. This module is that discoverable home: every consumer imports
//! from here on equal terms, and no package is privileged.

use super::*;
use crate::target::shared::abi;

/// Hex-encode `text` as a NUL-terminated C string payload (two hex digits per
/// byte, then `00`). Used to lay down read-only C-string data objects (library
/// sonames, `dlsym` names, framework paths).
pub(super) fn hex_encode_cstring(text: &str) -> String {
    let mut hex = String::new();
    for byte in text.bytes() {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.push_str("00"); // NUL terminator
    hex
}

/// Load the address of a read-only data symbol into `dst` (adrp + add).
pub(super) fn emit_data_address(
    from: &str,
    dst: &str,
    data_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", dst)
            .field("symbol", data_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", dst)
            .field("src", dst)
            .field("symbol", data_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: data_symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: data_symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

/// `bl _mfb_arena_free` returning a single compiler-sized block to the arena.
/// The caller stages the block pointer in the return register (`x0`) and its
/// original allocation size in `ARG[1]` (`x1`).
pub(super) fn emit_arena_free(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(ARENA_FREE_SYMBOL));
    relocations.push(super::internal_branch(symbol, ARENA_FREE_SYMBOL));
}

/// Emit the standard `Result` error tail: move `code` into the value register
/// and the ERR tag into the tag register, push the error-message data address,
/// and branch to `done`.
pub(super) fn emit_fail(
    symbol: &str,
    code: &str,
    message_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", code),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, message_symbol, instructions, relocations);
    instructions.push(abi::branch(done));
}

#[allow(clippy::too_many_arguments)]
/// Read a `List OF Byte` (collection pointer already stored at `coll_off`) into a
/// freshly arena-allocated contiguous buffer. Stores the buffer pointer at
/// `buf_off` and the byte count at `len_off`. Uses only vreg scratch (no calls).
/// Branches to `alloc_fail` on allocation failure.
pub(super) fn emit_read_byte_list(
    symbol: &str,
    tag: &str,
    coll_off: usize,
    buf_off: usize,
    len_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let copy_loop = format!("{symbol}_{tag}_read_loop");
    let copy_done = format!("{symbol}_{tag}_read_done");
    // count = coll->count; allocate max(count,1) bytes.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), coll_off),
        abi::load_u64("%v10", "%v9", COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", abi::stack_pointer(), len_off),
        abi::add_immediate(abi::return_register(), "%v10", 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), buf_off),
        // dataBase = coll + HEADER + capacity*ENTRY_SIZE
        abi::load_u64("%v9", abi::stack_pointer(), coll_off),
        abi::load_u64("%v11", "%v9", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v12", "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers("%v13", "%v11", "%v12"),
        abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v13", "%v9", "%v13"), // %v13 = dataBase
        abi::add_immediate("%v14", "%v9", COLLECTION_HEADER_SIZE), // %v14 = entry cursor
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::load_u64("%v15", abi::stack_pointer(), buf_off), // out cursor
        abi::move_immediate("%v9", "Integer", "0"),           // i
        abi::label(&copy_loop),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_eq(&copy_done),
        // byte = dataBase[entry->value_offset]; for kind 2 element `i` is simply
        // at offset `i`, with no entry to indirect through (plan-57-D). An `if`
        // expression rather than a split `extend` keeps the emitted array the
        // same shape, so the kind-0 build is untouched.
        if byte_list_entry_stride() != 0 {
            abi::load_u64("%v16", "%v14", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET)
        } else {
            abi::move_register("%v16", "%v9")
        },
        abi::add_registers("%v16", "%v13", "%v16"),
        abi::load_u8("%v17", "%v16", 0),
        abi::store_u8("%v17", "%v15", 0),
        abi::add_immediate("%v15", "%v15", 1),
        abi::add_immediate("%v14", "%v14", byte_list_entry_stride()),
        abi::add_immediate("%v9", "%v9", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
    ]);
}

#[allow(clippy::too_many_arguments)]
/// Build a `List OF Byte` of `len_off` bytes copied from the contiguous buffer at
/// `src_off`, storing the collection pointer at `coll_off`. Uses only vreg
/// scratch. Branches to `alloc_fail` on allocation failure.
/// Allocate a `List OF Byte` of `len_off` elements, write its header, and fill
/// the lookup table with the identity mapping while copying the payload bytes
/// from `src_off` — entry write and byte copy fused in one pass.
///
/// The single `List OF Byte` constructor for the socket/TLS/EC/entropy
/// backends. It is one of the places that must stop writing a lookup table when
/// plan-57-D gives a fixed-width list no entry array; consolidating the copies
/// here is what keeps that a single edit.
///
/// Two knobs, because that is the whole of the variation between the sites:
///
/// - `block` is the register holding the freshly allocated block. `net/io`
///   moves the allocator result into a vreg first (plan-34-B Phase 3); the TLS
///   and EC paths address `abi::RET[1]` directly. When `block` is not
///   `abi::RET[1]` the move is emitted here.
/// - `coll_off` is `Some` only where the caller wants the block pointer spilled
///   to a frame slot. The TLS paths keep it in the register and never spill.
///
/// `entry_loop`/`entry_done` are the caller's own label names rather than being
/// derived here, for the same reason: the sites had different naming schemes and
/// a rename would show up as a real diff in the generated dump.
///
/// All three knobs exist so every caller reproduces its previous instruction
/// stream exactly. `scripts/artifact-gate.sh` covers all of them across
/// macos-aarch64 / linux-aarch64 / linux-x86_64.
pub(super) fn emit_build_byte_list(
    symbol: &str,
    entry_loop: &str,
    entry_done: &str,
    src_off: usize,
    len_off: usize,
    coll_off: Option<usize>,
    block: &str,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // size = HEADER + count*ENTRY_SIZE + count(data)
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::move_immediate("%v11", "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "%v12", "%v10"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    // `block` names the register the freshly allocated block lives in, which is
    // wherever `emit_alloc` left it. Parameterizing it (plan-57-B) left behind a
    // `block != block` guard that was meant to move the pointer when a caller
    // asked for a different register — always false, so it never emitted
    // anything, and all four callers pass the allocation register anyway. State
    // the contract instead of pretending to handle the other case: a caller that
    // genuinely wants a different register has to emit that move deliberately.
    debug_assert_eq!(
        block,
        abi::RET[1],
        "emit_build_byte_list writes through the allocation's return register"
    );
    if let Some(slot) = coll_off {
        instructions.push(abi::store_u64(block, abi::stack_pointer(), slot));
    }
    instructions.extend([
        abi::move_immediate("%v9", "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8("%v9", block, COLLECTION_OFFSET_KIND),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("%v9", block, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8("%v9", block, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("%v9", "Byte", "1"),
        abi::store_u8("%v9", block, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::store_u64("%v10", block, COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", block, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("%v10", block, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v10", block, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("%v11", block, COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v12", "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers("%v13", "%v10", "%v12"),
        abi::add_registers("%v14", "%v11", "%v13"), // data base
        abi::load_u64("%v15", abi::stack_pointer(), src_off),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::label(entry_loop),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_eq(entry_done),
        // kind 2 has no entry array to fill (plan-57-D). Emitting this with a
        // zero stride would rewrite one entry over the data region `count`
        // times and run past the block, so it is skipped outright.
    ]);
    if byte_list_entry_stride() != 0 {
        instructions.extend([
            abi::move_immediate("%v12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8("%v12", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64("%v9", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate("%v12", "Integer", "1"),
            abi::store_u64("%v12", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        ]);
    }
    // The payload copy runs for BOTH representations — only the entry-field
    // stores above are kind-0 only.
    instructions.extend([
        abi::add_registers("%v12", "%v14", "%v9"),
        abi::load_u8("%v13", "%v15", 0),
        abi::store_u8("%v13", "%v12", 0),
        abi::add_immediate("%v15", "%v15", 1),
        abi::add_immediate("%v11", "%v11", byte_list_entry_stride()),
        abi::add_immediate("%v9", "%v9", 1),
        abi::branch(entry_loop),
        abi::label(entry_done),
    ]);
}

/// Overwrite the buffer at `[buf_off]` (length `[len_off]` when `Some`, else the
/// constant `len_const`) with zero, when the buffer slot is non-NULL. Wipes raw
/// key-material scratch (SEC1/PKCS#8 DER, raw scalar copies, entropy buffers)
/// before the helper returns so a later same-program arena allocation cannot be
/// handed a block still holding key bytes (bug-55, bug-177 D). Call-free (vreg
/// scratch only). `tag` disambiguates the labels per call site.
pub(super) fn emit_zero_guarded(
    symbol: &str,
    buf_off: usize,
    len_off: Option<usize>,
    len_const: usize,
    tag: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let skip = format!("{symbol}_{tag}_noz");
    let loop_l = format!("{symbol}_{tag}_zl");
    let end_l = format!("{symbol}_{tag}_ze");
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), buf_off),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&skip),
    ]);
    match len_off {
        Some(off) => ins.push(abi::load_u64("%v10", abi::stack_pointer(), off)),
        None => ins.push(abi::move_immediate(
            "%v10",
            "Integer",
            &len_const.to_string(),
        )),
    }
    ins.extend([
        abi::move_immediate("%v11", "Integer", "0"),
        abi::label(&loop_l),
        abi::compare_registers("%v11", "%v10"),
        abi::branch_eq(&end_l),
        abi::store_u8(abi::ZERO, "%v9", 0),
        abi::add_immediate("%v9", "%v9", 1),
        abi::add_immediate("%v11", "%v11", 1),
        abi::branch(&loop_l),
        abi::label(&end_l),
        abi::label(&skip),
    ]);
}
