// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::debug::arena::{
    emit_debug_arena_add, emit_debug_arena_bump, emit_debug_arena_live_add,
    emit_debug_arena_live_sub, emit_debug_arena_slot, COUNTER_ALLOC_BYTES, COUNTER_ALLOC_CALLS,
    COUNTER_DOUBLE_FREE_SKIPS, COUNTER_FILL_FREE_BYTES, COUNTER_FILL_FREE_CALLS, COUNTER_FLUSHES,
    COUNTER_FREE_BYTES, COUNTER_FREE_CALLS, COUNTER_GROW, COUNTER_HIT_CARVE, COUNTER_HIT_LARGE_BIN,
    COUNTER_HIT_QUICK_BIN, COUNTER_HIT_WALK, COUNTER_INSERT_FREE_CALLS, COUNTER_MAPPED_BYTES,
    COUNTER_MAPS, COUNTER_UNMAPPED_BYTES, COUNTER_UNMAPS, COUNTER_ZERO_CHECKS,
    COUNTER_ZERO_VIOLATIONS, DEBUG_ARENA_SAMPLE_SYMBOL,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

mod alloc;
mod free;
pub(crate) use alloc::{lower_arena_alloc, lower_simd_alloc_list};
pub(crate) use free::{lower_arena_flush_coalesce, lower_arena_free, lower_arena_insert_free};
/// Emit `perf_start(name)` / `perf_end(name)` at an arena-region boundary: load the
/// region's name object into the arg register and `bl` the perf helper. The
/// register allocator spills any live vreg across the `bl` (perf clobbers x0–x17),
/// and perf is arena-free by construction, so bracketing arena code cannot recurse
/// (perf → arena → perf). `from` is the emitting helper's symbol.
fn emit_perf_arena_call(
    call: &str,
    from: &str,
    name_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let sym = crate::target::shared::runtime::symbol_for_call(
        crate::target::shared::runtime::RuntimeHelper::Perf,
        call,
    );
    push_symbol_address(from, name_symbol, abi::c_arg(0), instructions, relocations);
    instructions.push(abi::branch_link(&sym));
    relocations.push(internal_branch(from, &sym));
}

/// bug-690: `slot = state + class(size) * 8`, the large-bin slot a chunk of
/// normalized `size` (> `ARENA_QUICK_BIN_MAX`) parks on; its list head is the word at
/// `slot + ARENA_LARGE_BIN_BASE_OFFSET`.
///
/// The classes are log2 size classes with `2^ARENA_LARGE_BIN_SUBCLASS_BITS` linear
/// sub-classes per power of two. With 4 sub-classes, for `2^k ≤ size < 2^(k+1)` the
/// class is `(k - 11) * 4 + ((size >> (k - 2)) & 3)`: class 0 is `[2048, 2560)`, and
/// every size in a class is smaller than every size in any higher class. Sizes from
/// `ARENA_LARGE_BIN_TOP_SIZE` up share the last class. That ordering is what lets an
/// allocation take ANY chunk from a higher class without looking at its size.
///
/// `k`/`shift`/`sub` are scratch vregs the caller owns; `prefix` makes the two
/// labels unique within the calling helper.
fn emit_large_bin_slot(
    slot: impl Into<abi::Reg>,
    size: impl Into<abi::Reg>,
    k: impl Into<abi::Reg>,
    shift: impl Into<abi::Reg>,
    sub: impl Into<abi::Reg>,
    prefix: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let slot: abi::Reg = slot.into();
    let size: abi::Reg = size.into();
    let k: abi::Reg = k.into();
    let shift: abi::Reg = shift.into();
    let sub: abi::Reg = sub.into();
    let top = format!("{prefix}_top");
    let ready = format!("{prefix}_ready");
    let sub_bits = ARENA_LARGE_BIN_SUBCLASS_BITS;
    let base_k = ARENA_QUICK_BIN_MAX.trailing_zeros();
    let top_k = ARENA_LARGE_BIN_TOP_SIZE.trailing_zeros();
    instructions.extend([
        // k = floor(log2(size)) = 63 - clz(size); size > QUICK_BIN_MAX, so k ≥ base_k.
        abi::count_leading_zeros(k, size),
        abi::move_immediate(shift, "Integer", "63"),
        abi::subtract_registers(k, shift, k),
        abi::compare_immediate(k, &(top_k - 1).to_string()),
        abi::branch_hi(&top),
        // sub = the `sub_bits` bits just below the leading one.
        abi::subtract_immediate(shift, k, sub_bits as usize),
        abi::shift_right_variable(sub, size, shift),
        abi::move_immediate(shift, "Integer", &((1u64 << sub_bits) - 1).to_string()),
        abi::and_registers(sub, sub, shift),
        // class = (k << sub_bits) + sub - (base_k << sub_bits).
        abi::shift_left_immediate(k, k, sub_bits as u8),
        abi::add_registers(sub, sub, k),
        abi::subtract_immediate(sub, sub, (base_k << sub_bits) as usize),
        abi::branch(&ready),
        abi::label(&top),
        abi::move_immediate(sub, "Integer", &(ARENA_LARGE_BIN_COUNT - 1).to_string()),
        abi::label(&ready),
        abi::shift_left_immediate(sub, sub, 3),
        abi::add_registers(slot, ARENA_STATE_REGISTER, sub),
    ]);
}

/// bug-690: park the free chunk `ptr`/`size` (normalized, ≥ 16) on its bin — the
/// exact quick bin for `size ≤ ARENA_QUICK_BIN_MAX`, else its large-bin size class —
/// writing the `FreeNode {next, size}` overlay. `slot`/`head`/`k`/`shift`/`sub` are
/// scratch vregs the caller owns; `prefix` makes the labels unique in the helper.
#[allow(clippy::too_many_arguments)]
fn emit_park_chunk(
    ptr: impl Into<abi::Reg>,
    size: impl Into<abi::Reg>,
    slot: impl Into<abi::Reg>,
    head: impl Into<abi::Reg>,
    k: impl Into<abi::Reg>,
    shift: impl Into<abi::Reg>,
    sub: impl Into<abi::Reg>,
    prefix: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let ptr: abi::Reg = ptr.into();
    let size: abi::Reg = size.into();
    let slot: abi::Reg = slot.into();
    let head: abi::Reg = head.into();
    let k: abi::Reg = k.into();
    let shift: abi::Reg = shift.into();
    let sub: abi::Reg = sub.into();
    let large = format!("{prefix}_large");
    let done = format!("{prefix}_done");
    instructions.extend([
        abi::store_u64(size, ptr, 8),
        abi::compare_immediate(size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi(&large),
        abi::shift_right_immediate(slot, size, 4),
        abi::shift_left_immediate(slot, slot, 3),
        abi::add_registers(slot, ARENA_STATE_REGISTER, slot),
        abi::load_u64(head, slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(head, ptr, 0),
        abi::store_u64(ptr, slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch(&done),
        abi::label(&large),
    ]);
    emit_large_bin_slot(
        slot,
        size,
        k,
        shift,
        sub,
        &format!("{prefix}_class"),
        instructions,
    );
    instructions.extend([
        abi::load_u64(head, slot, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::store_u64(head, ptr, 0),
        abi::store_u64(ptr, slot, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::label(&done),
    ]);
}

/// plan-130-C: a successful allocation of `size` served by the path whose hit counter
/// is `counter` — bump it and raise `live_bytes`/`peak_live_bytes`. No-op without a
/// registry slot (a normal build, or an unregistered arena).
fn emit_debug_alloc_hit(
    slot: &Option<VirtualRegister>,
    counter: usize,
    size: impl Into<abi::Reg>,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let size: abi::Reg = size.into();
    if let Some(slot) = slot {
        emit_debug_arena_bump(ARENA_ALLOC_SYMBOL, slot, counter, tag, instructions, vregs);
        emit_debug_arena_live_add(
            ARENA_ALLOC_SYMBOL,
            slot,
            size,
            &format!("{tag}_live"),
            instructions,
            vregs,
        );
    }
}

/// `arena_fill_zero(x0 = ptr, x1 = len)` — overwrite `len` bytes at `ptr` with zero.
/// `ptr` is 8-aligned and `len` a multiple of 8 (every caller hands it a 16-byte-aligned
/// chunk whose size is a multiple of 16). Leaf (makes no call); vreg-allocated, so it
/// clobbers caller-saved registers only.
///
/// It scrubs a freed chunk. One overwrite destroys the old value in RAM, whatever the
/// new value is, and zero is the cheapest to write. Freshly mapped memory is never
/// handed to it: the OS returns it zeroed (plan-183-A). The body is per target:
///
/// * **AArch64**: when the startup probe left a block size in `_mfb_rt_dczva_bytes`
///   (`DCZID_EL0` permits `dc zva`), plain stores up to block alignment, one `dc zva`
///   per block, then the generic loop for the tail. A range shorter than two blocks,
///   or a zero word, goes straight to the generic loop.
/// * **x86-64**: one `rep stosb`.
/// * **riscv64** (and AArch64 without `dc zva`): the generic loop — eight 8-byte zero
///   stores, 64 bytes an iteration, then one word at a time.
pub(crate) fn lower_arena_fill_zero(arch: &str) -> CodeFunction {
    let mut vregs = Vregs::new();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    if arch == "x86_64" {
        let ptr = vregs.next();
        let len = vregs.next();
        instructions.extend([
            abi::move_register(&ptr, abi::c_arg(0)),
            abi::move_register(&len, abi::c_arg(1)),
            abi::rep_stosb_zero(&ptr, &len),
            abi::return_(),
        ]);
        return finalize_vreg_helper(
            "runtime.arena_fill_zero",
            ARENA_FILL_ZERO_SYMBOL,
            "Nothing",
            instructions,
            relocations,
        );
    }
    let ptr = vregs.next();
    let end = vregs.next();
    let remaining = vregs.next();
    instructions.extend([
        abi::move_register(&ptr, abi::c_arg(0)),
        abi::add_registers(&end, abi::c_arg(0), abi::c_arg(1)),
    ]);
    if arch == "aarch64" {
        let block = vregs.next();
        let mask = vregs.next();
        let scratch = vregs.next();
        push_symbol_address(
            ARENA_FILL_ZERO_SYMBOL,
            DCZVA_BYTES_SYMBOL,
            &scratch,
            &mut instructions,
            &mut relocations,
        );
        instructions.extend([
            abi::load_u64(&block, &scratch, 0),
            abi::compare_immediate(&block, "0"),
            abi::branch_eq("arena_fill_generic"),
            // At least two blocks, so the aligned run holds one whole block.
            abi::subtract_registers(&remaining, &end, &ptr),
            abi::shift_left_immediate(&scratch, &block, 1),
            abi::compare_registers(&remaining, &scratch),
            abi::branch_lo("arena_fill_generic"),
            abi::subtract_immediate(&mask, &block, 1),
            abi::label("arena_fill_zva_head"),
            abi::and_registers(&scratch, &ptr, &mask),
            abi::compare_immediate(&scratch, "0"),
            abi::branch_eq("arena_fill_zva_aligned"),
            abi::store_u64(abi::ZERO, &ptr, 0),
            abi::add_immediate(&ptr, &ptr, 8),
            abi::branch("arena_fill_zva_head"),
            abi::label("arena_fill_zva_aligned"),
            // The block run stops at the last block boundary at or below `end`.
            abi::bitwise_not(&scratch, &mask),
            abi::and_registers(&scratch, &end, &scratch),
            abi::label("arena_fill_zva_loop"),
            abi::dc_zva(&ptr),
            abi::add_registers(&ptr, &ptr, &block),
            abi::compare_registers(&ptr, &scratch),
            abi::branch_lo("arena_fill_zva_loop"),
        ]);
    }
    instructions.extend([
        abi::label("arena_fill_generic"),
        abi::subtract_registers(&remaining, &end, &ptr),
        abi::compare_immediate(&remaining, "64"),
        abi::branch_lo("arena_fill_tail"),
    ]);
    for offset in (0..64).step_by(8) {
        instructions.push(abi::store_u64(abi::ZERO, &ptr, offset));
    }
    instructions.extend([
        abi::add_immediate(&ptr, &ptr, 64),
        abi::branch("arena_fill_generic"),
        abi::label("arena_fill_tail"),
        abi::compare_registers(&ptr, &end),
        abi::branch_lo("arena_fill_word"),
        abi::return_(),
        abi::label("arena_fill_word"),
        abi::store_u64(abi::ZERO, &ptr, 0),
        abi::add_immediate(&ptr, &ptr, 8),
        abi::branch("arena_fill_tail"),
    ]);
    finalize_vreg_helper(
        "runtime.arena_fill_zero",
        ARENA_FILL_ZERO_SYMBOL,
        "Nothing",
        instructions,
        relocations,
    )
}

pub(crate) fn lower_arena_destroy(
    platform: &dyn CodegenPlatform,
    debug_arena: bool,
    // plan-179-C: give each unmapped block's bytes back to the arena memory cap.
    arena_cap: bool,
    platform_imports: &std::collections::HashMap<String, String>,
) -> Result<CodeFunction, String> {
    // Vreg-allocated (plan-00-G Phase 2): walk the block list and `munmap` each
    // block. `head` (the loop cursor) and `next` are loop-carried across the
    // `munmap` syscall, so the allocator keeps them in callee-saved registers (or
    // spills them); the syscall's own ABI registers (x0/x1 + the syscall-number
    // register) stay physical. The block address/size are passed to the syscall in
    // x0/x1, exactly where `emit_arena_unmap` expects them.
    let mut vregs = Vregs::new();
    let head = vregs.next();
    let next = vregs.next();
    let clear_cursor = vregs.next();
    let clear_limit = vregs.next();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // plan-130-C: find this arena's registry slot once; each block it unmaps counts,
    // read before the unmap's argument registers are loaded.
    let mut dbg_slot: Option<VirtualRegister> = None;
    if debug_arena {
        let slot = vregs.next();
        emit_debug_arena_slot(
            ARENA_DESTROY_SYMBOL,
            &slot,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        );
        dbg_slot = Some(slot);
    }
    instructions.extend([
        abi::load_u64(&head, ARENA_STATE_REGISTER, 0),
        abi::label("arena_destroy_loop"),
        abi::compare_immediate(&head, "0"),
        abi::branch_eq("arena_destroy_done"),
        abi::load_u64(&next, &head, 0),
    ]);
    if let Some(slot) = &dbg_slot {
        let block_size = vregs.next();
        instructions.push(abi::load_u64(&block_size, &head, 8));
        emit_debug_arena_bump(
            ARENA_DESTROY_SYMBOL,
            slot,
            COUNTER_UNMAPS,
            "unmaps",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_arena_add(
            ARENA_DESTROY_SYMBOL,
            slot,
            COUNTER_UNMAPPED_BYTES,
            &block_size,
            "unmapped_bytes",
            &mut instructions,
            &mut vregs,
        );
    }
    if arena_cap {
        let block_size = vregs.next();
        instructions.push(abi::load_u64(&block_size, &head, 8));
        crate::codegen::runtime::arena_cap::emit_unmap_refund(
            &mut crate::codegen::runtime::arena_cap::CapSite {
                symbol: ARENA_DESTROY_SYMBOL,
                platform,
                platform_imports,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &block_size,
            &mut vregs,
        )?;
    }
    instructions.extend([
        abi::load_u64(abi::c_arg(1), &head, 8),
        abi::move_register(abi::return_register(), &head),
    ]);
    platform.emit_arena_unmap(&mut instructions)?;
    instructions.extend([
        abi::move_register(&head, &next),
        abi::branch("arena_destroy_loop"),
        abi::label("arena_destroy_done"),
        // Leave the arena fully inert (allocator-04): clear the free-list head
        // alongside the block-list head — it points into the just-unmapped
        // blocks, and a stale head would turn any post-destroy allocation into
        // a use-after-free walk. The quick bins (allocator-01) point into the
        // same unmapped blocks, so clear them too.
        abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, 0),
        abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::add_immediate(
            &clear_cursor,
            ARENA_STATE_REGISTER,
            ARENA_QUICK_BIN_BASE_OFFSET,
        ),
        abi::add_immediate(&clear_limit, ARENA_STATE_REGISTER, ARENA_STATE_SIZE),
        abi::label("arena_destroy_bins"),
        abi::store_u64(abi::ZERO, &clear_cursor, 0),
        abi::add_immediate(&clear_cursor, &clear_cursor, 8),
        abi::compare_registers(&clear_cursor, &clear_limit),
        abi::branch_lo("arena_destroy_bins"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "runtime.arena_destroy",
        ARENA_DESTROY_SYMBOL,
        "Nothing",
        instructions,
        relocations,
    ))
}

#[cfg(test)]
mod zero_fill_tests {
    use super::*;
    use crate::arch::ops::CodeOp;

    /// The fill writes zero and nothing else: every store's source is the zero
    /// register, it reads no arena state (offsets 16/24 are reserved), and it neither
    /// multiplies nor calls. plan-183-A: its one load is the AArch64 `dc zva` block size,
    /// its one data reference that word's address; riscv64 loads and references nothing.
    #[test]
    fn the_arena_fill_stores_only_zero_words() {
        for (arch, backend) in [
            (
                "aarch64",
                &crate::arch::aarch64::backend::AARCH64_BACKEND
                    as &dyn crate::codegen::engine::mir::Backend,
            ),
            ("riscv64", &crate::arch::riscv64::backend::RISCV64_BACKEND),
        ] {
            crate::codegen::engine::mir::mir::set_backend(backend);
            let fill = lower_arena_fill_zero(arch);
            let zero = abi::store_u64(abi::ZERO, abi::SCRATCH[0], 0)
                .get("src")
                .expect("a store names its source");
            let stores: Vec<_> = fill
                .instructions
                .iter()
                .filter(|i| i.op == CodeOp::StrU64)
                .collect();
            assert!(
                stores.len() >= 9,
                "{arch}: the eight-word loop and the one-word tail"
            );
            for store in &stores {
                assert_eq!(
                    store.get("src").as_deref(),
                    Some(zero.as_str()),
                    "{arch}: the fill stores something other than zero"
                );
            }
            let loads = fill
                .instructions
                .iter()
                .filter(|i| i.get("dst").is_some() && i.get("base").is_some())
                .count();
            let (want_loads, want_refs): (usize, &[&str]) = if arch == "aarch64" {
                (1, &[DCZVA_BYTES_SYMBOL])
            } else {
                (0, &[])
            };
            assert_eq!(
                loads, want_loads,
                "{arch}: the fill reads only the dc zva word"
            );
            assert!(
                fill.relocations
                    .iter()
                    .all(|r| want_refs.contains(&r.to.as_str())),
                "{arch}: the fill is a leaf: no call, no data but the dc zva word"
            );
            assert_eq!(
                fill.instructions.iter().any(|i| i.op == CodeOp::DcZva),
                arch == "aarch64",
                "{arch}: dc zva is the AArch64 body's block zero"
            );
        }
    }

    /// plan-183-A: x86-64 zeroes the whole range with one `rep stosb` and nothing else.
    #[test]
    fn the_x86_fill_is_one_rep_stosb() {
        crate::codegen::engine::mir::mir::set_backend(
            &crate::arch::x86_64::backend::X86_64_BACKEND,
        );
        let fill = lower_arena_fill_zero("x86_64");
        assert_eq!(
            fill.instructions
                .iter()
                .filter(|i| i.op == CodeOp::RepStosbZero)
                .count(),
            1
        );
        assert!(fill.instructions.iter().all(|i| i.op != CodeOp::StrU64));
        assert!(fill.relocations.is_empty(), "a leaf with no data reference");
    }

    /// Freed chunks are scrubbed by the fill, and nothing else: plan-183-A's grow path
    /// hands out freshly mapped memory as the OS zeroed it, with no fill and no
    /// random-stream helper left anywhere in the allocator.
    #[test]
    fn freed_chunks_are_zero_filled_and_fresh_blocks_are_not() {
        crate::codegen::engine::mir::mir::set_backend(
            &crate::arch::aarch64::backend::AARCH64_BACKEND,
        );
        let alloc = lower_arena_alloc(
            &crate::target::macos_aarch64::code::Platform,
            false,
            false,
            false,
            &std::collections::HashMap::new(),
        )
        .expect("arena_alloc");
        let free = lower_arena_free(false, false);
        assert!(
            free.relocations
                .iter()
                .any(|r| r.to == ARENA_FILL_ZERO_SYMBOL),
            "arena_free does not zero-fill"
        );
        assert!(
            alloc
                .relocations
                .iter()
                .all(|r| r.to != ARENA_FILL_ZERO_SYMBOL),
            "arena_alloc fills fresh memory, which the OS already zeroed"
        );
        for (name, function) in [("arena_alloc", &alloc), ("arena_free", &free)] {
            assert!(
                function
                    .relocations
                    .iter()
                    .all(|r| !r.to.contains("fill_random")
                        && !r.to.contains("fill_seed")
                        && !r.to.contains("fill_next")),
                "{name} still references a random-fill helper"
            );
        }
    }
}
