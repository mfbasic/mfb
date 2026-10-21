// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::debug::arena::{
    emit_debug_arena_add, emit_debug_arena_bump, emit_debug_arena_live_add,
    emit_debug_arena_live_sub, emit_debug_arena_slot, COUNTER_ALLOC_BYTES, COUNTER_ALLOC_CALLS,
    COUNTER_DOUBLE_FREE_SKIPS, COUNTER_FILL_FREE_BYTES, COUNTER_FILL_FREE_CALLS,
    COUNTER_FILL_GROW_BYTES, COUNTER_FILL_GROW_CALLS, COUNTER_FLUSHES, COUNTER_FREE_BYTES,
    COUNTER_FREE_CALLS, COUNTER_GROW, COUNTER_HIT_CARVE, COUNTER_HIT_LARGE_BIN,
    COUNTER_HIT_QUICK_BIN, COUNTER_HIT_WALK, COUNTER_INSERT_FREE_CALLS, COUNTER_MAPPED_BYTES,
    COUNTER_MAPS, COUNTER_UNMAPPED_BYTES, COUNTER_UNMAPS, DEBUG_ARENA_SAMPLE_SYMBOL,
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

/// `arena_fill_zero(x0 = ptr, x1 = len)` — overwrite `len` bytes at `ptr` with zero,
/// `len` rounded up to a whole word. Every chunk handed to it is a multiple of 16 bytes,
/// so the rounding is exact and never writes past the chunk. Four words a loop, then one
/// at a time. Leaf (makes no call); vreg-allocated, so it clobbers caller-saved
/// registers only.
///
/// It scrubs a freed chunk and fills a freshly mapped block. One overwrite destroys the
/// old value in RAM, whatever the new value is, and zero is the cheapest to write. It
/// replaced a PCG64 fill (bug-769's unrolled stream) that cost a 128-bit multiply a word
/// and bought no more confidentiality.
pub(crate) fn lower_arena_fill_zero() -> CodeFunction {
    let mut vregs = Vregs::new();
    let ptr = vregs.next();
    let count = vregs.next();
    let instructions = vec![
        abi::label("entry"),
        abi::move_register(&ptr, abi::c_arg(0)),
        abi::move_register(&count, abi::c_arg(1)),
        // word count = (len + 7) >> 3
        abi::add_immediate(&count, &count, 7),
        abi::shift_right_immediate(&count, &count, 3),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq("arena_fill_done"),
        abi::compare_immediate(&count, "4"),
        abi::branch_lt("arena_fill_tail"),
        abi::label("arena_fill_block"),
        abi::store_u64(abi::ZERO, &ptr, 0),
        abi::store_u64(abi::ZERO, &ptr, 8),
        abi::store_u64(abi::ZERO, &ptr, 16),
        abi::store_u64(abi::ZERO, &ptr, 24),
        abi::add_immediate(&ptr, &ptr, 32),
        abi::subtract_immediate(&count, &count, 4),
        abi::compare_immediate(&count, "4"),
        abi::branch_ge("arena_fill_block"),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq("arena_fill_done"),
        abi::label("arena_fill_tail"),
        abi::store_u64(abi::ZERO, &ptr, 0),
        abi::add_immediate(&ptr, &ptr, 8),
        abi::subtract_immediate(&count, &count, 1),
        abi::compare_immediate(&count, "0"),
        abi::branch_ne("arena_fill_tail"),
        abi::label("arena_fill_done"),
        abi::return_(),
    ];
    finalize_vreg_helper(
        "runtime.arena_fill_zero",
        ARENA_FILL_ZERO_SYMBOL,
        "Nothing",
        instructions,
        Vec::new(),
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

    fn select_backend() {
        crate::codegen::engine::mir::mir::set_backend(
            &crate::arch::aarch64::backend::AARCH64_BACKEND,
        );
    }

    /// The fill writes zero and nothing else: every store's source is the zero
    /// register, it loads nothing (so it reads no arena state — offsets 16/24 are
    /// reserved), and it neither multiplies nor calls.
    #[test]
    fn the_arena_fill_stores_only_zero_words() {
        select_backend();
        let fill = lower_arena_fill_zero();
        let zero = abi::store_u64(abi::ZERO, abi::SCRATCH[0], 0)
            .get("src")
            .expect("a store names its source");
        let stores: Vec<_> = fill
            .instructions
            .iter()
            .filter(|i| i.get("src").is_some() && i.get("base").is_some())
            .collect();
        assert!(
            stores.len() >= 5,
            "the four-word loop and the one-word tail"
        );
        for store in &stores {
            assert_eq!(
                store.get("src").as_deref(),
                Some(zero.as_str()),
                "the fill stores something other than zero"
            );
        }
        assert!(
            fill.instructions
                .iter()
                .all(|i| !(i.get("dst").is_some() && i.get("base").is_some())),
            "the fill loads: it must read nothing, the arena state least of all"
        );
        assert!(
            fill.relocations.is_empty(),
            "the fill is a leaf: no call, no data reference"
        );
    }

    /// Both users of the fill call it, and nothing else: a freed chunk is scrubbed and a
    /// freshly mapped block is zeroed before first use, with no random-stream helper
    /// left anywhere in the allocator.
    #[test]
    fn fresh_blocks_and_freed_chunks_are_zero_filled() {
        select_backend();
        let alloc = lower_arena_alloc(
            &crate::target::macos_aarch64::code::Platform,
            false,
            false,
            false,
            &std::collections::HashMap::new(),
        )
        .expect("arena_alloc");
        let free = lower_arena_free(false, false);
        for (name, function) in [("arena_alloc", &alloc), ("arena_free", &free)] {
            assert!(
                function
                    .relocations
                    .iter()
                    .any(|r| r.to == ARENA_FILL_ZERO_SYMBOL),
                "{name} does not zero-fill"
            );
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
        // `arena_alloc` fills on its grow path only: a fresh block, before first use.
        let calls: Vec<usize> = alloc
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, i)| i.get("target").as_deref() == Some(ARENA_FILL_ZERO_SYMBOL))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(
            calls.len(),
            1,
            "arena_alloc zero-fills on exactly one path, the grow"
        );
    }
}
