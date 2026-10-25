//! plan-183-B: the arena reservation — one range of address space reserved at startup
//! and cut into equal arena slices.
//!
//! `_mfb_rt_arena_reserve` runs once from the program entry, after the cap check and
//! before the first allocation. It reserves `slices × 20 GiB` (plus 2 MiB, so the
//! slices can start 2 MiB-aligned) with no access, halving on failure down to the floor
//! of `slices × 256 MiB`; below it the program writes one line to stderr and exits 1.
//! Every arena then commits its memory from its own slice: slice 0 is the entry
//! thread's, slice 1 a canvas program's graphics thread's, and each `thread::start`
//! takes two free ones (its worker arena and its control-block arena) under
//! `_mfb_rt_arena_slice_lock`, and gives them back when its control block is freed.
//!
//! Slices = `1 + (1 if the program uses canvas) + 2 × options.maxThreads`, fixed at
//! build time. All slices are equal: `min(20 GiB, reserved ÷ slices)`, rounded down to
//! 2 MiB.

use super::*;
use crate::codegen::error::constants::*;
use crate::target::shared::nir::NirModule;

/// The process-wide reservation base (u64): slice `i` starts at `base + i × sliceSize`.
pub(crate) const ARENA_BASE_SYMBOL: &str = "_mfb_rt_arena_base";
/// The bytes of one slice (u64).
pub(crate) const ARENA_SLICE_SIZE_SYMBOL: &str = "_mfb_rt_arena_slice_size";
/// The bytes the reservation holds, alignment slack included (u64).
pub(crate) const ARENA_RESERVED_BYTES_SYMBOL: &str = "_mfb_rt_arena_reserved_bytes";
/// One bit per slice, set while the slice belongs to an arena; guarded by
/// `_mfb_rt_arena_slice_lock` (`ceil(slices / 64)` words).
pub(crate) const ARENA_SLICE_BITS_SYMBOL: &str = "_mfb_rt_arena_slice_bits";
/// The startup helper.
pub(crate) const ARENA_RESERVE_SYMBOL: &str = "_mfb_rt_arena_reserve";
const S_RESERVE_FAILED: &str = "_mfb_str_arena_reserve_failed";

/// The largest slice.
pub(crate) const ARENA_SLICE_MAX: u64 = 20 << 30;
/// The smallest slice the reservation may settle for.
pub(crate) const ARENA_SLICE_FLOOR: u64 = 256 << 20;
/// Slice alignment, and the reservation's alignment slack.
pub(crate) const ARENA_SLICE_ALIGN: u64 = 2 << 20;
/// The granule every commit and decommit is rounded to: a multiple of every target's
/// page (4 KiB, macOS AArch64's 16 KiB, Windows' 64 KiB allocation granularity).
pub(crate) const ARENA_COMMIT_GRANULE: u64 = 64 << 10;

/// The slice of the entry thread's arena.
pub(crate) const MAIN_SLICE: u64 = 0;
/// The slice of a canvas program's graphics-thread arena.
pub(crate) const GRAPHICS_SLICE: u64 = 1;

/// How many slices `module` reserves: the entry thread's, the graphics thread's in a
/// canvas program, and two per allowed thread.
pub(crate) fn arena_slice_count(module: &NirModule) -> u64 {
    1 + u64::from(crate::codegen::engine::analysis::module_analysis::module_uses_canvas(module))
        + 2 * module.arena_memory.max_threads
}

/// The slices no `thread::start` may take: the entry thread's and, in a canvas
/// program, the graphics thread's.
pub(crate) fn fixed_slice_count(module: &NirModule) -> u64 {
    1 + u64::from(crate::codegen::engine::analysis::module_analysis::module_uses_canvas(module))
}

fn bitmap_words(slices: u64) -> u64 {
    slices.div_ceil(64)
}

/// The reservation's words and its failure line. Emitted for every native module:
/// `arena_alloc` reads the slice size in each.
pub(crate) fn data_objects_reservation(module: &NirModule) -> Vec<CodeDataObject> {
    let slices = arena_slice_count(module);
    let word = |symbol: &str, layout: &str| CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: layout.to_string(),
        align: 8,
        size: 8,
        value: "00".repeat(8),
    };
    let words = bitmap_words(slices) as usize;
    vec![
        word(ARENA_BASE_SYMBOL, "mfb.runtime.arena_base.v1 { u64 base }"),
        word(
            ARENA_SLICE_SIZE_SYMBOL,
            "mfb.runtime.arena_slice_size.v1 { u64 sliceBytes }",
        ),
        word(
            ARENA_RESERVED_BYTES_SYMBOL,
            "mfb.runtime.arena_reserved_bytes.v1 { u64 reservedBytes }",
        ),
        CodeDataObject {
            symbol: ARENA_SLICE_BITS_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: format!("mfb.runtime.arena_slice_bits.v1 {{ u64 bits[{words}] }}"),
            align: 8,
            size: words * 8,
            value: "00".repeat(words * 8),
        },
        string_data_object(
            S_RESERVE_FAILED,
            format!(
                "error: could not reserve {} bytes of address space for {slices} arenas; \
                 the program did not start.\n",
                slices * ARENA_SLICE_FLOOR + ARENA_SLICE_ALIGN
            ),
        ),
    ]
}

/// `_mfb_rt_arena_reserve`: reserve the slices, record the base and slice size, mark
/// the fixed slices taken, and point the entry thread's arena (the arena register) at
/// slice 0. Returns 0 to run the program, 1 to stop it with exit status 1.
pub(crate) fn lower_arena_reserve(
    module: &NirModule,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let slices = arena_slice_count(module);
    let fixed = fixed_slice_count(module);
    let symbol = ARENA_RESERVE_SYMBOL;
    let mut vregs = Vregs::new();
    let size = vregs.next();
    let request = vregs.next();
    let floor = vregs.next();
    let base = vregs.next();
    let scratch = vregs.next();
    let addr = vregs.next();
    let try_label = format!("{symbol}_try");
    let reserved = format!("{symbol}_reserved");
    let failed = format!("{symbol}_failed");
    let clamp_done = format!("{symbol}_clamp_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_immediate(&size, "Integer", &(slices * ARENA_SLICE_MAX).to_string()),
        abi::move_immediate(&floor, "Integer", &(slices * ARENA_SLICE_FLOOR).to_string()),
        abi::move_immediate(&addr, "Integer", "0"),
        abi::label(&try_label),
        abi::add_immediate(&request, &size, ARENA_SLICE_ALIGN as usize),
    ];
    let mut relocations = Vec::new();
    platform.emit_arena_vm(
        ArenaVm::Reserve,
        (&addr).into(),
        (&request).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&base, abi::return_register()),
        abi::compare_immediate(&base, "0"),
        abi::branch_ge(&reserved),
        // Refused: the floor was the last try; otherwise halve, settling for exactly the
        // floor when half would fall below it.
        abi::compare_registers(&size, &floor),
        abi::branch_ls(&failed),
        abi::shift_right_immediate(&size, &size, 1),
        abi::compare_registers(&size, &floor),
        abi::branch_hi(&clamp_done),
        abi::move_register(&size, &floor),
        abi::label(&clamp_done),
        abi::branch(&try_label),
        abi::label(&reserved),
    ]);
    // base = round_up(base, 2 MiB); sliceSize = min(20 GiB, size / slices) & ~(2 MiB - 1).
    let store = |instructions: &mut Vec<CodeInstruction>,
                 relocations: &mut Vec<CodeRelocation>,
                 symbol_of: &str,
                 value: &VirtualRegister,
                 scratch: &VirtualRegister| {
        push_symbol_address(symbol, symbol_of, scratch, instructions, relocations);
        instructions.push(abi::store_u64(value, scratch, 0));
    };
    store(
        &mut instructions,
        &mut relocations,
        ARENA_RESERVED_BYTES_SYMBOL,
        &request,
        &scratch,
    );
    instructions.extend([
        abi::add_immediate(&base, &base, (ARENA_SLICE_ALIGN - 1) as usize),
        abi::move_immediate(&scratch, "Integer", &(!(ARENA_SLICE_ALIGN - 1)).to_string()),
        abi::and_registers(&base, &base, &scratch),
        abi::move_immediate(&scratch, "Integer", &slices.to_string()),
        abi::unsigned_divide_registers(&size, &size, &scratch),
        abi::move_immediate(&scratch, "Integer", &ARENA_SLICE_MAX.to_string()),
        abi::compare_registers(&size, &scratch),
        abi::branch_ls(&format!("{symbol}_slice_capped")),
        abi::move_register(&size, &scratch),
        abi::label(&format!("{symbol}_slice_capped")),
        abi::move_immediate(&scratch, "Integer", &(!(ARENA_SLICE_ALIGN - 1)).to_string()),
        abi::and_registers(&size, &size, &scratch),
    ]);
    store(
        &mut instructions,
        &mut relocations,
        ARENA_BASE_SYMBOL,
        &base,
        &scratch,
    );
    store(
        &mut instructions,
        &mut relocations,
        ARENA_SLICE_SIZE_SYMBOL,
        &size,
        &scratch,
    );
    // The fixed slices are taken for good: bits 0 .. fixed - 1 of word 0.
    let taken = vregs.next();
    instructions.push(abi::move_immediate(
        &taken,
        "Integer",
        &((1u64 << fixed) - 1).to_string(),
    ));
    store(
        &mut instructions,
        &mut relocations,
        ARENA_SLICE_BITS_SYMBOL,
        &taken,
        &scratch,
    );
    // The entry thread's arena: slice 0, nothing committed yet.
    instructions.extend([
        abi::store_u64(&base, ARENA_STATE_REGISTER, ARENA_SLICE_BASE_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
        abi::label(&failed),
    ]);
    push_symbol_address(
        symbol,
        S_RESERVE_FAILED,
        &scratch,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(abi::string_length_register(), &scratch, 0),
        abi::add_immediate(abi::string_data_register(), &scratch, 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);
    debug_assert_eq!(MAIN_SLICE, 0);
    Ok(finalize_vreg_helper(
        "runtime.arena_reserve",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}
