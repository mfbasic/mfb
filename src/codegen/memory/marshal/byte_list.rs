//! Split from `the retired flat native_helpers.rs` (category `memory.marshal`).

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

/// `N` vregs the caller's body has not used. Every helper here is spliced into a
/// caller's instruction stream, so it must never name a vreg itself: the caller's
/// `Vregs` numbers from `%v0`, and a fixed `%vN` would be whichever of the
/// caller's values carries that number (bug-564).
fn mint<const N: usize>(vregs: &mut Vregs) -> [VirtualRegister; N] {
    std::array::from_fn(|_| vregs.next())
}
#[allow(clippy::too_many_arguments)]
/// Read a `List OF Byte` (collection pointer already stored at `coll_off`) into a
/// freshly arena-allocated contiguous buffer. Stores the buffer pointer at
/// `buf_off` and the byte count at `len_off`. Uses only vreg scratch (no calls).
/// Branches to `alloc_fail` on allocation failure.
pub(crate) fn emit_read_byte_list(
    symbol: &str,
    tag: &str,
    coll_off: usize,
    buf_off: usize,
    len_off: usize,
    alloc_fail: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let [coll, count, cap, stride, base, entry, out, index, at, byte] = mint(vregs);
    let copy_loop = format!("{symbol}_{tag}_read_loop");
    let copy_done = format!("{symbol}_{tag}_read_done");
    // count = coll->count; allocate max(count,1) bytes.
    instructions.extend([
        abi::load_u64(&coll, abi::stack_pointer(), coll_off),
        abi::load_u64(&count, &coll, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, abi::stack_pointer(), len_off),
        abi::add_immediate(abi::return_register(), &count, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), buf_off),
        // dataBase = coll + HEADER + capacity*ENTRY_SIZE
        abi::load_u64(&coll, abi::stack_pointer(), coll_off),
        abi::load_u64(&cap, &coll, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(&stride, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&base, &cap, &stride),
        abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE),
        abi::add_registers(&base, &coll, &base), // dataBase
        abi::add_immediate(&entry, &coll, COLLECTION_HEADER_SIZE), // entry cursor
        abi::load_u64(&count, abi::stack_pointer(), len_off),
        abi::load_u64(&out, abi::stack_pointer(), buf_off), // out cursor
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &count),
        abi::branch_eq(&copy_done),
        // byte = dataBase[entry->value_offset]; for kind 2 element `i` is simply
        // at offset `i`, with no entry to indirect through (plan-57-D). An `if`
        // expression rather than a split `extend` keeps the emitted array the
        // same shape, so the kind-0 build is untouched.
        if byte_list_entry_stride() != 0 {
            abi::load_u64(&at, &entry, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET)
        } else {
            abi::move_register(&at, &index)
        },
        abi::add_registers(&at, &base, &at),
        abi::load_u8(&byte, &at, 0),
        abi::store_u8(&byte, &out, 0),
        abi::add_immediate(&out, &out, 1),
        abi::add_immediate(&entry, &entry, byte_list_entry_stride()),
        abi::add_immediate(&index, &index, 1),
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
///   and EC paths address `abi::mfb_return(1)` directly. When `block` is not
///   `abi::mfb_return(1)` the move is emitted here.
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
pub(crate) fn emit_build_byte_list(
    symbol: &str,
    entry_loop: &str,
    entry_done: &str,
    src_off: usize,
    len_off: usize,
    coll_off: Option<usize>,
    block: impl Into<abi::Reg>,
    alloc_fail: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let block = block.into();
    let [count, stride, size, byte, entry, data, src, index, at, value] = mint(vregs);
    // size = HEADER + count*ENTRY_SIZE + count(data)
    instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), len_off),
        abi::move_immediate(&stride, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&size, &count, &stride),
        abi::add_immediate(&size, &size, COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), &size, &count),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    // `block` names the register the freshly allocated block lives in, which is
    // wherever `emit_alloc` left it. Parameterizing it (plan-57-B) left behind a
    // `block != block` guard that was meant to move the pointer when a caller
    // asked for a different register — always false, so it never emitted
    // anything, and all four callers pass the allocation register anyway. State
    // the contract instead of pretending to handle the other case: a caller that
    // genuinely wants a different register has to emit that move deliberately.
    assert_eq!(
        block,
        abi::mfb_return(1),
        "emit_build_byte_list writes through the allocation's return register"
    );
    if let Some(slot) = coll_off {
        instructions.push(abi::store_u64(&block, abi::stack_pointer(), slot));
    }
    instructions.extend([
        abi::move_immediate(&byte, "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(&byte, &block, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&byte, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&byte, &block, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&byte, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(&byte, &block, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&byte, "Byte", "1"),
        abi::store_u8(&byte, &block, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64(&count, abi::stack_pointer(), len_off),
        abi::store_u64(&count, &block, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &block, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&count, &block, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&count, &block, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&entry, &block, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&stride, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&data, &count, &stride),
        abi::add_registers(&data, &entry, &data), // data base
        abi::load_u64(&src, abi::stack_pointer(), src_off),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(entry_loop),
        abi::compare_registers(&index, &count),
        abi::branch_eq(entry_done),
        // kind 2 has no entry array to fill (plan-57-D). Emitting this with a
        // zero stride would rewrite one entry over the data region `count`
        // times and run past the block, so it is skipped outright.
    ]);
    if byte_list_entry_stride() != 0 {
        instructions.extend([
            abi::move_immediate(&value, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8(&value, &entry, COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, &entry, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, &entry, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64(&index, &entry, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate(&value, "Integer", "1"),
            abi::store_u64(&value, &entry, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        ]);
    }
    // The payload copy runs for BOTH representations — only the entry-field
    // stores above are kind-0 only.
    instructions.extend([
        abi::add_registers(&at, &data, &index),
        abi::load_u8(&value, &src, 0),
        abi::store_u8(&value, &at, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&entry, &entry, byte_list_entry_stride()),
        abi::add_immediate(&index, &index, 1),
        abi::branch(entry_loop),
        abi::label(entry_done),
    ]);
}

/// Overwrite the buffer at `[buf_off]` (length `[len_off]` when `Some`, else the
/// constant `len_const`) with zero, when the buffer slot is non-NULL. Wipes raw
/// key-material scratch (SEC1/PKCS#8 DER, raw scalar copies, entropy buffers)
/// before the helper returns so a later same-program arena allocation cannot be
/// handed a block still holding key bytes (bug-55, bug-177 D). Call-free; its
/// scratch vregs are minted from the caller's `vregs`. `tag` disambiguates the labels per call site.
pub(crate) fn emit_zero_guarded(
    symbol: &str,
    buf_off: usize,
    len_off: Option<usize>,
    len_const: usize,
    tag: &str,
    vregs: &mut Vregs,
    ins: &mut Vec<CodeInstruction>,
) {
    let [cursor, len, index] = mint(vregs);
    let skip = format!("{symbol}_{tag}_noz");
    let loop_l = format!("{symbol}_{tag}_zl");
    let end_l = format!("{symbol}_{tag}_ze");
    ins.extend([
        abi::load_u64(&cursor, abi::stack_pointer(), buf_off),
        abi::compare_immediate(&cursor, "0"),
        abi::branch_eq(&skip),
    ]);
    match len_off {
        Some(off) => ins.push(abi::load_u64(&len, abi::stack_pointer(), off)),
        None => ins.push(abi::move_immediate(&len, "Integer", &len_const.to_string())),
    }
    ins.extend([
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&loop_l),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&end_l),
        abi::store_u8(abi::ZERO, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&loop_l),
        abi::label(&end_l),
        abi::label(&skip),
    ]);
}

/// Free the `List OF Byte` whose pointer is at `coll_off` — a block
/// [`emit_build_byte_list`] built with its element count at `len_off` — when the slot is
/// non-null, then null the slot so a second call frees nothing.
///
/// With `wipe`, the whole block is zeroed first: a caller that has byte-copied key
/// material out of the list must not hand those bytes back to a later same-program
/// allocation (bug-55, bug-177 D). The size given to `_mfb_arena_free` is the builder's own
/// `count·stride + HEADER + count`, because the free must receive the allocation's size
/// (bug-560). The free is a PCS call that clobbers every caller-saved register, so the
/// caller keeps anything live (the result value) in a frame slot across it (bug-625).
pub(crate) fn emit_free_byte_list_guarded(
    symbol: &str,
    tag: &str,
    coll_off: usize,
    len_off: usize,
    wipe: bool,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let [block, count, stride, size, index, at] = mint(vregs);
    let skip = format!("{symbol}_{tag}_free_skip");
    let block_size = |instructions: &mut Vec<CodeInstruction>| {
        instructions.extend([
            abi::load_u64(&count, abi::stack_pointer(), len_off),
            abi::move_immediate(&stride, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&size, &count, &stride),
            abi::add_immediate(&size, &size, COLLECTION_HEADER_SIZE),
            abi::add_registers(&size, &size, &count),
        ]);
    };
    instructions.extend([
        abi::load_u64(&block, abi::stack_pointer(), coll_off),
        abi::compare_immediate(&block, "0"),
        abi::branch_eq(&skip),
    ]);
    if wipe {
        let wipe_loop = format!("{symbol}_{tag}_free_wipe");
        let wipe_done = format!("{symbol}_{tag}_free_wiped");
        block_size(instructions);
        instructions.extend([
            abi::move_immediate(&index, "Integer", "0"),
            abi::label(&wipe_loop),
            abi::compare_registers(&index, &size),
            abi::branch_eq(&wipe_done),
            abi::add_registers(&at, &block, &index),
            abi::store_u8(abi::ZERO, &at, 0),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&wipe_loop),
            abi::label(&wipe_done),
        ]);
    }
    block_size(instructions);
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), coll_off),
        abi::move_register(abi::c_arg(0), &block),
        abi::move_register(abi::c_arg(1), &size),
    ]);
    emit_arena_free(symbol, instructions, relocations);
    instructions.push(abi::label(&skip));
}

/// The byte count a scratch buffer was allocated with: read from a frame slot, or a
/// compile-time constant.
pub(crate) enum ScratchSize {
    Slot(usize),
    Bytes(usize),
}

/// Free the contiguous scratch buffer whose pointer is at `ptr_off` — allocated with
/// `size` bytes, which is the size `_mfb_arena_free` must receive (bug-560) — when the
/// slot is non-null, then null the slot so a second call frees nothing. The free is a
/// PCS call that clobbers every caller-saved register, so the caller keeps anything
/// live (the result value) in a frame slot across it (bug-625).
///
/// It works in the free's own argument registers and names NO vreg. It used to stage
/// the pointer in a hard-coded `%v9`, and a body that mints its vregs from `%v0` owns a
/// `%v9` of its own: Schannel `tls::connect`'s host-string `HelperScratch` pointer was
/// exactly that vreg, so this free overwrote it, the release at `done` freed the wrong
/// block with the wrong size, and the next `tls::connect` crashed in `_mfb_arena_alloc`
/// (bug-564). The call clobbers these registers anyway, so staging there costs nothing.
pub(crate) fn emit_free_buffer_guarded(
    symbol: &str,
    tag: &str,
    ptr_off: usize,
    size: ScratchSize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let skip = format!("{symbol}_{tag}_buf_free_skip");
    instructions.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), ptr_off),
        abi::compare_immediate(abi::c_arg(0), "0"),
        abi::branch_eq(&skip),
        match size {
            ScratchSize::Slot(len_off) => {
                abi::load_u64(abi::c_arg(1), abi::stack_pointer(), len_off)
            }
            ScratchSize::Bytes(bytes) => {
                abi::move_immediate(abi::c_arg(1), "Integer", &bytes.to_string())
            }
        },
        abi::store_u64(abi::ZERO, abi::stack_pointer(), ptr_off),
    ]);
    emit_arena_free(symbol, instructions, relocations);
    instructions.push(abi::label(&skip));
}

#[cfg(test)]
mod tests {
    // bug-564: a marshal helper is spliced INTO its caller's body, and that body
    // numbers its own vregs from `%v0` with a `Vregs`. A helper that names a fixed
    // `%vN` therefore writes whichever caller vreg happens to carry that number.
    // `emit_free_buffer_guarded`'s `%v9` was Schannel `tls::connect`'s live
    // host-string pointer, and the second connect in a Windows process crashed.
    //
    // Two pins. The scan below keeps the whole module free of literal vreg names
    // (a new helper cannot reintroduce the class by copy-paste). The behavioural
    // test drives every helper with a caller that already holds 32 vregs and
    // asserts no emitted operand is one of them.
    use super::*;
    use crate::codegen::engine::mir;
    use crate::codegen::engine::util::Vregs;

    /// Every `"%v<digits>"` string literal in `source` outside its test module.
    fn literal_vreg_names(source: &str) -> Vec<String> {
        let code = source.split("#[cfg(test)]").next().unwrap_or(source);
        let mut found = Vec::new();
        let mut rest = code;
        while let Some(at) = rest.find("\"%v") {
            let tail = &rest[at + 3..];
            let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() && tail[digits.len()..].starts_with('"') {
                found.push(format!("%v{digits}"));
            }
            rest = &rest[at + 3..];
        }
        found
    }

    #[test]
    fn no_marshal_helper_names_a_fixed_vreg() {
        for (file, source) in [
            ("byte_list.rs", include_str!("byte_list.rs")),
            ("record.rs", include_str!("record.rs")),
            ("record_list.rs", include_str!("record_list.rs")),
        ] {
            let names = literal_vreg_names(source);
            assert!(
                names.is_empty(),
                "{file} names {} fixed vreg(s) outside its tests ({:?}…); a helper spliced \
                 into a caller's body must mint its vregs from the caller's `Vregs`",
                names.len(),
                &names[..names.len().min(6)]
            );
        }
    }

    /// Every rendered operand of every instruction.
    fn operands(instructions: &[CodeInstruction]) -> Vec<String> {
        instructions
            .iter()
            .flat_map(|i| {
                i.fields
                    .iter()
                    .map(|(key, _)| i.get(key).unwrap_or_default())
            })
            .collect()
    }

    #[test]
    fn every_helper_mints_its_vregs_from_the_caller() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let mut vregs = Vregs::new();
        let held: Vec<String> = (0..32).map(|_| vregs.next().to_string()).collect();
        let mut ins = Vec::new();
        let mut rel = Vec::new();
        emit_read_byte_list("s", "r", 8, 16, 24, "f", &mut vregs, &mut ins, &mut rel);
        emit_build_byte_list(
            "s",
            "bl",
            "bd",
            16,
            24,
            Some(32),
            abi::mfb_return(1),
            "f",
            &mut vregs,
            &mut ins,
            &mut rel,
        );
        emit_zero_guarded("s", 16, Some(24), 0, "z", &mut vregs, &mut ins);
        emit_zero_guarded("s", 16, None, 64, "zc", &mut vregs, &mut ins);
        emit_free_byte_list_guarded("s", "w", 32, 24, true, &mut vregs, &mut ins, &mut rel);
        emit_free_byte_list_guarded("s", "n", 32, 24, false, &mut vregs, &mut ins, &mut rel);
        emit_free_buffer_guarded("s", "b", 16, ScratchSize::Slot(24), &mut ins, &mut rel);
        let used = operands(&ins);
        assert!(
            used.iter().any(|o| o.starts_with("%v")),
            "the helpers must emit vregs at all, or this test cannot fail"
        );
        for operand in used {
            assert!(
                !held.contains(&operand),
                "a marshal helper wrote `{operand}`, a vreg its caller already held"
            );
        }
    }
}
