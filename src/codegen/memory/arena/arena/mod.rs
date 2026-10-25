// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::debug::arena::{
    emit_debug_arena_add, emit_debug_arena_bump, emit_debug_arena_live_add,
    emit_debug_arena_live_sub, emit_debug_arena_slot, COUNTER_ALLOC_BYTES, COUNTER_ALLOC_CALLS,
    COUNTER_COMMITS, COUNTER_COMMITTED_BYTES, COUNTER_COMMITTED_PEAK_BYTES, COUNTER_DECOMMITS,
    COUNTER_DECOMMITTED_BYTES, COUNTER_DOUBLE_FREE_SKIPS, COUNTER_FILL_FREE_BYTES,
    COUNTER_FILL_FREE_CALLS, COUNTER_FREE_BYTES, COUNTER_FREE_CALLS, COUNTER_HIT_HUGE_FRONTIER,
    COUNTER_HIT_HUGE_RUN, COUNTER_HIT_MEDIUM_FRONTIER, COUNTER_HIT_MEDIUM_RUN, COUNTER_HIT_SMALL,
    COUNTER_RESERVED_BYTES, COUNTER_SMALL_PAGES, COUNTER_ZERO_CHECKS, COUNTER_ZERO_VIOLATIONS,
    DEBUG_ARENA_SAMPLE_SYMBOL,
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
mod extend;
mod free;
mod layout;
mod small;
mod span;
pub(crate) use alloc::{lower_arena_alloc, lower_simd_alloc_list};
pub(crate) use extend::{lower_arena_extend, ARENA_EXTEND_SYMBOL};
pub(crate) use free::lower_arena_free;
pub(crate) use layout::{lower_arena_commit, lower_arena_geometry};
pub(crate) use small::lower_arena_small_page;
pub(crate) use span::{lower_arena_span_alloc, lower_arena_span_free};
/// Emit `perf_start(name)` / `perf_end(name)` at an arena-region boundary: load the
/// region's name object into the arg register and `bl` the perf helper. The
/// register allocator spills any live vreg across the `bl` (perf clobbers x0–x17),
/// and perf is arena-free by construction, so bracketing arena code cannot recurse
/// (perf → arena → perf). `from` is the emitting helper's symbol.
pub(crate) fn emit_perf_arena_call(
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

/// `slot = state + class(size) * 8`: the medium list a run of normalized `size`
/// (> `ARENA_SMALL_MAX`) belongs to has its head at `slot +
/// ARENA_MEDIUM_HEAD_BASE_OFFSET` (bug-690's size classes, kept by plan-183-C).
///
/// The classes are log2 size classes with `2^ARENA_MEDIUM_SUBCLASS_BITS` linear
/// sub-classes per power of two. With 4 sub-classes, for `2^k ≤ size < 2^(k+1)` the
/// class is `(k - 11) * 4 + ((size >> (k - 2)) & 3)`: class 0 is `[2048, 2560)`, and
/// every size in a class is smaller than every size in any higher class. Sizes from
/// `ARENA_MEDIUM_TOP_SIZE` up share the last class. That ordering is what lets an
/// allocation take ANY run from a higher class without looking at its size.
///
/// `k`/`shift`/`sub` are scratch vregs the caller owns; `prefix` makes the two
/// labels unique within the calling helper.
pub(crate) fn emit_medium_class_slot(
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
    let sub_bits = ARENA_MEDIUM_SUBCLASS_BITS;
    let base_k = ARENA_SMALL_MAX.trailing_zeros();
    let top_k = ARENA_MEDIUM_TOP_SIZE.trailing_zeros();
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
        abi::move_immediate(sub, "Integer", &(ARENA_MEDIUM_CLASS_COUNT - 1).to_string()),
        abi::label(&ready),
        abi::shift_left_immediate(sub, sub, 3),
        abi::add_registers(slot, ARENA_STATE_REGISTER, sub),
    ]);
}

/// plan-130-C: a successful allocation of `size` served by the path whose hit counter
/// is `counter` — bump it and raise `live_bytes`/`peak_live_bytes`. No-op without a
/// registry slot (a normal build, or an unregistered arena).
pub(crate) fn emit_debug_alloc_hit(
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

/// `arena_destroy()` — give back everything the arena (the arena register) holds:
/// plan-183-C decommits its whole slice in one call (its pages go back to the OS and
/// read zero when committed again), refunds its committed bytes to the arena memory
/// cap, and clears its allocator words — the region geometry, the page and run lists,
/// the node pool — so the arena is inert and lays its slice out afresh if it
/// allocates again. The slice stays the arena's; `thread::start`'s bookkeeping hands
/// worker and control-block slices back. Idempotent: an arena that never allocated
/// (or was already destroyed) has nothing to give back.
pub(crate) fn lower_arena_destroy(
    platform: &dyn CodegenPlatform,
    debug_arena: bool,
    arena_cap: bool,
    platform_imports: &std::collections::HashMap<String, String>,
) -> Result<CodeFunction, String> {
    let symbol = ARENA_DESTROY_SYMBOL;
    let mut vregs = Vregs::new();
    let sc = vregs.next();
    let base = vregs.next();
    let len = vregs.next();
    let committed = vregs.next();
    let cursor = vregs.next();
    let limit = vregs.next();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    layout::emit_scalars(&sc, &mut instructions);
    instructions.extend([
        abi::load_u64(&len, &sc, ARENA_SC_SMALL_PAGES),
        abi::compare_immediate(&len, "0"),
        abi::branch_eq("arena_destroy_done"),
        abi::load_u64(&base, ARENA_STATE_REGISTER, ARENA_SLICE_BASE_OFFSET),
        abi::load_u64(&len, &sc, ARENA_SC_SLICE_TOP),
        abi::subtract_registers(&len, &len, &base),
        abi::load_u64(&committed, &sc, ARENA_SC_COMMITTED),
    ]);
    platform.emit_arena_vm(
        ArenaVm::Decommit,
        (&base).into(),
        (&len).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if arena_cap {
        crate::codegen::runtime::arena_cap::emit_decommit_refund(
            &mut crate::codegen::runtime::arena_cap::CapSite {
                symbol,
                platform,
                platform_imports,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &committed,
            &mut vregs,
        )?;
    }
    if debug_arena {
        let slot = vregs.next();
        emit_debug_arena_slot(
            symbol,
            &slot,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        );
        emit_debug_arena_bump(
            symbol,
            &slot,
            COUNTER_DECOMMITS,
            "decommits",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_arena_add(
            symbol,
            &slot,
            COUNTER_DECOMMITTED_BYTES,
            &committed,
            "decommitted",
            &mut instructions,
            &mut vregs,
        );
        instructions.extend([
            abi::compare_immediate(&slot, "0"),
            abi::branch_eq("arena_destroy_dbg_skip"),
            abi::store_u64(abi::ZERO, &slot, COUNTER_COMMITTED_BYTES),
            abi::label("arena_destroy_dbg_skip"),
        ]);
    }
    // Clear the lists (small and medium heads, the huge head, the node free list) and
    // the scalar block.
    instructions.extend([
        abi::add_immediate(&cursor, ARENA_STATE_REGISTER, ARENA_SMALL_HEAD_BASE_OFFSET),
        abi::add_immediate(&limit, ARENA_STATE_REGISTER, ARENA_NODE_FREE_OFFSET + 8),
        abi::label("arena_destroy_clear_small"),
        abi::store_u64(abi::ZERO, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::compare_registers(&cursor, &limit),
        abi::branch_lo("arena_destroy_clear_small"),
        abi::add_immediate(&cursor, ARENA_STATE_REGISTER, ARENA_MEDIUM_HEAD_BASE_OFFSET),
        abi::add_immediate(
            &limit,
            ARENA_STATE_REGISTER,
            ARENA_MEDIUM_HEAD_BASE_OFFSET + ARENA_MEDIUM_CLASS_COUNT * 8,
        ),
        abi::label("arena_destroy_clear_medium"),
        abi::store_u64(abi::ZERO, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::compare_registers(&cursor, &limit),
        abi::branch_lo("arena_destroy_clear_medium"),
        abi::move_register(&cursor, &sc),
        abi::add_immediate(&limit, &sc, ARENA_ALLOC_SCALARS_SIZE),
        abi::label("arena_destroy_clear_scalars"),
        abi::store_u64(abi::ZERO, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::compare_registers(&cursor, &limit),
        abi::branch_lo("arena_destroy_clear_scalars"),
        abi::label("arena_destroy_done"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "runtime.arena_destroy",
        symbol,
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

    /// Freed chunks are scrubbed by the fill — small slots by `arena_free`, medium and
    /// huge chunks by `_mfb_arena_span_free` — and nothing else: freshly committed
    /// memory is handed out as the OS zeroed it (plan-183-A), with no fill and no
    /// random-stream helper left anywhere in the allocator.
    #[test]
    fn freed_chunks_are_zero_filled_and_fresh_blocks_are_not() {
        crate::codegen::engine::mir::mir::set_backend(
            &crate::arch::aarch64::backend::AARCH64_BACKEND,
        );
        let alloc = lower_arena_alloc(false, false);
        let free = lower_arena_free(false, false);
        let span_free = lower_arena_span_free(false);
        for (name, function) in [("arena_free", &free), ("arena_span_free", &span_free)] {
            assert!(
                function
                    .relocations
                    .iter()
                    .any(|r| r.to == ARENA_FILL_ZERO_SYMBOL),
                "{name} does not zero-fill"
            );
        }
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
