//! plan-183-C: the slice geometry and committing.
//!
//! An arena owns one slice `[base, base + S)` of the startup reservation (S at least
//! 256 MiB, a multiple of 2 MiB). Inside it:
//!
//! ```text
//! | small: S/8                  | tags | nodes | medium → grows up     ... grows down ← huge |
//! | descriptors | 64 KiB pages  |      side    |                                             |
//! ```
//!
//! - **Small**: `ARENA_SMALL_DESC_SIZE`-byte page descriptors, then 64 KiB pages,
//!   each serving one 16-byte size class.
//! - **Tags**: one 16-byte record `{start, end}` per 2 KiB block of the medium and
//!   huge span: `start` describes what begins in the block (an allocated chunk:
//!   `size | offset << 48 | 1`; a free run: `node << 2 | 2`), `end` names the free
//!   run whose last byte lies in it.
//! - **Nodes**: the free runs, `{start, size, next, prev}`, out of free memory.
//! - **Medium** grows up from its base, **huge** down from the slice top.
//!
//! The geometry words are computed from the slice base the first time the arena
//! allocates (`_mfb_arena_geometry`), so an arena that never allocates touches none
//! of this. Each region commits on demand — data in `ARENA_COMMIT_STEP`s, the side
//! structures in `ARENA_COMMIT_GRANULE`s — through `_mfb_arena_commit`, which charges
//! the arena memory cap and counts the bytes.

use super::*;
use crate::codegen::runtime::arena_cap::{ARENA_COMMIT_GRANULE, ARENA_SLICE_SIZE_SYMBOL};

/// `_mfb_arena_geometry()`: compute this arena's region words from its slice base.
/// Returns 0, or 1 when the arena has no slice.
pub(crate) const ARENA_GEOMETRY_SYMBOL: &str = "_mfb_arena_geometry";
/// `_mfb_arena_commit(lo, hi)`: commit `[lo, hi)` (both granule-aligned, `lo < hi`)
/// and charge it. Returns 0, or 1 when the cap or the OS refuses.
pub(crate) const ARENA_COMMIT_SYMBOL: &str = "_mfb_arena_commit";

/// `dst = state + ARENA_ALLOC_SCALARS_OFFSET` — the base every scalar word is
/// addressed off (it lies past rv64's 12-bit immediate).
pub(crate) fn emit_scalars(dst: &VirtualRegister, instructions: &mut Vec<CodeInstruction>) {
    instructions.extend([
        abi::move_immediate(dst, "Integer", &ARENA_ALLOC_SCALARS_OFFSET.to_string()),
        abi::add_registers(dst, ARENA_STATE_REGISTER, dst),
    ]);
}

/// `dst = round_up(src, align)`; `scratch` is clobbered. `align` is a power of two.
pub(crate) fn emit_round_up(
    dst: &VirtualRegister,
    src: &VirtualRegister,
    align: u64,
    scratch: &VirtualRegister,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::move_immediate(scratch, "Integer", &(align - 1).to_string()),
        abi::add_registers(dst, src, scratch),
        abi::bitwise_not(scratch, scratch),
        abi::and_registers(dst, dst, scratch),
    ]);
}

/// `dst = round_down(src, align)`; `scratch` is clobbered.
pub(crate) fn emit_round_down(
    dst: &VirtualRegister,
    src: &VirtualRegister,
    align: u64,
    scratch: &VirtualRegister,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::move_immediate(scratch, "Integer", &(!(align - 1)).to_string()),
        abi::and_registers(dst, src, scratch),
    ]);
}

/// Make `[.., need)` committed for a region that grows **up**: if `need` is past the
/// committed end in scalar word `field`, commit up to `round_up(need, step)` — but
/// never past `ceiling` — and move the word. Branches to `fail` when the commit is
/// refused. `sc` is the scalars base. Clobbers caller-saved registers (a call).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_ensure_up(
    from: &str,
    tag: &str,
    sc: &VirtualRegister,
    field: usize,
    need: &VirtualRegister,
    ceiling: &VirtualRegister,
    step: u64,
    fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let top = vregs.next();
    let new = vregs.next();
    let scratch = vregs.next();
    let done = format!("{from}_{tag}_up_done");
    let capped = format!("{from}_{tag}_up_capped");
    instructions.extend([
        abi::load_u64(&top, sc, field),
        abi::compare_registers(need, &top),
        abi::branch_ls(&done),
    ]);
    emit_round_up(&new, need, step, &scratch, instructions);
    instructions.extend([
        abi::compare_registers(&new, ceiling),
        abi::branch_ls(&capped),
        abi::move_register(&new, ceiling),
        abi::label(&capped),
        abi::move_register(abi::c_arg(0), &top),
        abi::move_register(abi::c_arg(1), &new),
        abi::branch_link(ARENA_COMMIT_SYMBOL),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(fail),
        abi::store_u64(&new, sc, field),
        abi::label(&done),
    ]);
    relocations.push(internal_branch(from, ARENA_COMMIT_SYMBOL));
}

/// The **down**-growing counterpart of [`emit_ensure_up`]: commit from
/// `round_down(need, step)` (never below `floor`) up to the committed start in
/// `field`, and move the word down.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_ensure_down(
    from: &str,
    tag: &str,
    sc: &VirtualRegister,
    field: usize,
    need: &VirtualRegister,
    floor: &VirtualRegister,
    step: u64,
    fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let bottom = vregs.next();
    let new = vregs.next();
    let scratch = vregs.next();
    let done = format!("{from}_{tag}_down_done");
    let floored = format!("{from}_{tag}_down_floored");
    let below = format!("{from}_{tag}_down_below");
    let short = format!("{from}_{tag}_down_short");
    instructions.extend([
        abi::load_u64(&bottom, sc, field),
        abi::compare_registers(need, &bottom),
        abi::branch_lo(&below),
        abi::branch(&done),
        abi::label(&below),
    ]);
    emit_round_down(&new, need, step, &scratch, instructions);
    instructions.extend([
        abi::compare_registers(&new, floor),
        abi::branch_lo(&short),
        abi::branch(&floored),
        abi::label(&short),
        abi::move_register(&new, floor),
        abi::label(&floored),
        abi::move_register(abi::c_arg(0), &new),
        abi::move_register(abi::c_arg(1), &bottom),
        abi::branch_link(ARENA_COMMIT_SYMBOL),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(fail),
        abi::store_u64(&new, sc, field),
        abi::label(&done),
    ]);
    relocations.push(internal_branch(from, ARENA_COMMIT_SYMBOL));
}

/// `_mfb_arena_geometry`: lay the slice out (module docs) and store the region words.
pub(crate) fn lower_arena_geometry(debug_arena: bool) -> CodeFunction {
    let symbol = ARENA_GEOMETRY_SYMBOL;
    let mut vregs = Vregs::new();
    let sc = vregs.next();
    let base = vregs.next();
    let size = vregs.next();
    let top = vregs.next();
    let small_end = vregs.next();
    let pages = vregs.next();
    let t = vregs.next();
    let u = vregs.next();
    let blocks = vregs.next();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    emit_scalars(&sc, &mut instructions);
    instructions.extend([
        abi::load_u64(&base, ARENA_STATE_REGISTER, ARENA_SLICE_BASE_OFFSET),
        abi::compare_immediate(&base, "0"),
        abi::branch_eq("arena_geometry_none"),
    ]);
    push_symbol_address(
        symbol,
        ARENA_SLICE_SIZE_SYMBOL,
        &size,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&size, &size, 0),
        abi::add_registers(&top, &base, &size),
        abi::store_u64(&top, &sc, ARENA_SC_SLICE_TOP),
        // Small region: S/8. Descriptors for every page it could hold, then the pages.
        abi::shift_right_immediate(&small_end, &size, 3),
        abi::add_registers(&small_end, &base, &small_end),
        abi::store_u64(&small_end, &sc, ARENA_SC_SMALL_END),
        abi::shift_right_immediate(&pages, &size, 3 + ARENA_SMALL_PAGE.trailing_zeros() as u8),
        abi::move_immediate(&t, "Integer", &ARENA_SMALL_DESC_SIZE.to_string()),
        abi::multiply_registers(&t, &pages, &t),
        abi::add_registers(&t, &base, &t),
    ]);
    emit_round_up(&t, &t, ARENA_SMALL_PAGE, &u, &mut instructions);
    instructions.extend([
        abi::store_u64(&t, &sc, ARENA_SC_SMALL_PAGES),
        abi::store_u64(&t, &sc, ARENA_SC_SMALL_COMMIT),
        abi::store_u64(&base, &sc, ARENA_SC_TABLE_COMMIT),
        abi::subtract_registers(&u, &small_end, &t),
        abi::shift_right_immediate(&u, &u, ARENA_SMALL_PAGE.trailing_zeros() as u8),
        abi::store_u64(&u, &sc, ARENA_SC_SMALL_LIMIT),
        abi::store_u64(abi::ZERO, &sc, ARENA_SC_SMALL_NEXT),
        abi::store_u64(abi::ZERO, &sc, ARENA_SC_SMALL_EMPTY),
        // Side: one tag record per 2 KiB of the rest of the slice, then half as many
        // nodes (a free run and the chunk after it each span more than 2 KiB).
        abi::subtract_registers(&blocks, &top, &small_end),
        abi::shift_right_immediate(&blocks, &blocks, ARENA_TAG_BLOCK_SHIFT as u8),
        abi::store_u64(&small_end, &sc, ARENA_SC_TAGS),
        abi::store_u64(&small_end, &sc, ARENA_SC_TAGS_LOW),
        abi::shift_left_immediate(&t, &blocks, 4),
        abi::add_registers(&t, &small_end, &t),
    ]);
    emit_round_up(&t, &t, ARENA_COMMIT_GRANULE, &u, &mut instructions);
    instructions.extend([
        abi::store_u64(&t, &sc, ARENA_SC_TAGS_HIGH),
        abi::store_u64(&t, &sc, ARENA_SC_NODES),
        abi::store_u64(&t, &sc, ARENA_SC_NODE_COMMIT),
        abi::move_immediate(&u, "Integer", "1"),
        abi::store_u64(&u, &sc, ARENA_SC_NODE_NEXT),
        abi::shift_right_immediate(&u, &blocks, 1),
        abi::store_u64(&u, &sc, ARENA_SC_NODE_LIMIT),
        abi::shift_left_immediate(&u, &u, ARENA_NODE_SIZE.trailing_zeros() as u8),
        abi::add_registers(&t, &t, &u),
    ]);
    emit_round_up(&t, &t, ARENA_COMMIT_STEP, &u, &mut instructions);
    instructions.extend([
        abi::store_u64(&t, &sc, ARENA_SC_MEDIUM_BASE),
        abi::store_u64(&t, &sc, ARENA_SC_MEDIUM_FRONT),
        abi::store_u64(&t, &sc, ARENA_SC_MEDIUM_COMMIT),
        abi::store_u64(&top, &sc, ARENA_SC_HUGE_FRONT),
        abi::store_u64(&top, &sc, ARENA_SC_HUGE_COMMIT),
    ]);
    if debug_arena {
        let slot = vregs.next();
        emit_debug_arena_slot(
            symbol,
            &slot,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        );
        let skip = "arena_geometry_dbg_skip";
        instructions.extend([
            abi::compare_immediate(&slot, "0"),
            abi::branch_eq(skip),
            abi::store_u64(&size, &slot, COUNTER_RESERVED_BYTES),
            abi::label(skip),
        ]);
    }
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
        abi::label("arena_geometry_none"),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.arena_geometry",
        symbol,
        "Integer",
        instructions,
        relocations,
    )
}

/// `_mfb_arena_commit(lo, hi)`: charge `hi - lo` to the arena memory cap, commit the
/// range, and add it to the arena's committed bytes (and, in a `--debug` build, its
/// commit counters and memory series).
pub(crate) fn lower_arena_commit(
    platform: &dyn CodegenPlatform,
    debug_arena: bool,
    arena_cap: bool,
    platform_imports: &std::collections::HashMap<String, String>,
) -> Result<CodeFunction, String> {
    let symbol = ARENA_COMMIT_SYMBOL;
    let mut vregs = Vregs::new();
    let lo = vregs.next();
    let len = vregs.next();
    let charged = vregs.next();
    let sc = vregs.next();
    let total = vregs.next();
    let fail = "arena_commit_fail";
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&lo, abi::c_arg(0)),
        abi::subtract_registers(&len, abi::c_arg(1), abi::c_arg(0)),
        abi::move_immediate(&charged, "Integer", "0"),
    ];
    let mut relocations = Vec::new();
    if arena_cap {
        crate::codegen::runtime::arena_cap::emit_commit_charge(
            &mut crate::codegen::runtime::arena_cap::CapSite {
                symbol,
                platform,
                platform_imports,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &len,
            &charged,
            fail,
            &mut vregs,
        )?;
    }
    platform.emit_arena_vm(
        ArenaVm::Commit,
        (&lo).into(),
        (&len).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne("arena_commit_refused"),
    ]);
    emit_scalars(&sc, &mut instructions);
    instructions.extend([
        abi::load_u64(&total, &sc, ARENA_SC_COMMITTED),
        abi::add_registers(&total, &total, &len),
        abi::store_u64(&total, &sc, ARENA_SC_COMMITTED),
    ]);
    if debug_arena {
        let slot = vregs.next();
        let peak = vregs.next();
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
            COUNTER_COMMITS,
            "commits",
            &mut instructions,
            &mut vregs,
        );
        let skip = "arena_commit_dbg_skip";
        instructions.extend([
            abi::compare_immediate(&slot, "0"),
            abi::branch_eq(skip),
            abi::store_u64(&total, &slot, COUNTER_COMMITTED_BYTES),
            abi::load_u64(&peak, &slot, COUNTER_COMMITTED_PEAK_BYTES),
            abi::compare_registers(&total, &peak),
            abi::branch_ls(skip),
            abi::store_u64(&total, &slot, COUNTER_COMMITTED_PEAK_BYTES),
            abi::label(skip),
        ]);
    }
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
        abi::label("arena_commit_refused"),
    ]);
    if arena_cap {
        crate::codegen::runtime::arena_cap::emit_commit_refund(
            &mut crate::codegen::runtime::arena_cap::CapSite {
                symbol,
                platform,
                platform_imports,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &len,
            &charged,
            &mut vregs,
        )?;
    }
    instructions.extend([
        abi::label(fail),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "runtime.arena_commit",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}
