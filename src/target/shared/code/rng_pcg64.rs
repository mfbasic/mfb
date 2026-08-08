use super::*;

/// One PCG64 step `state = state * MULT + INC` (128-bit), over virtual registers.
/// `lo`/`hi` are the caller's state vregs (read and rewritten in place); all
/// scratch is freshly generated. The increment is added with explicit-carry
/// `add_carry` (the carry is a vreg value, not the flags register — plan-00-G §4),
/// so the chain survives register allocation.
fn emit_pcg_step(instructions: &mut Vec<CodeInstruction>, vregs: &mut Vregs, lo: &str, hi: &str) {
    let mult_lo = vregs.next();
    let prod_lo = vregs.next();
    let prod_hi = vregs.next();
    let cross_lo = vregs.next();
    let mult_hi = vregs.next();
    let cross_hi = vregs.next();
    let inc_lo = vregs.next();
    let inc_hi = vregs.next();
    let carry = vregs.next();
    instructions.extend([
        // 128-bit (truncated) product of state by the 128-bit multiplier.
        abi::move_immediate(&mult_lo, "Integer", &PCG_MULT_LO.to_string()),
        abi::multiply_registers(&prod_lo, &mult_lo, lo), // result low limb
        abi::unsigned_multiply_high_registers(&prod_hi, &mult_lo, lo), // carry into high
        abi::multiply_registers(&cross_lo, &mult_lo, hi), // MULT_LO * state_hi
        abi::move_immediate(&mult_hi, "Integer", &PCG_MULT_HI.to_string()),
        abi::multiply_registers(&cross_hi, &mult_hi, lo), // MULT_HI * state_lo
        abi::add_registers(&prod_hi, &prod_hi, &cross_lo),
        abi::add_registers(&prod_hi, &prod_hi, &cross_hi), // result high limb
        // Add the 128-bit increment with the carry as an explicit value.
        abi::move_immediate(&inc_lo, "Integer", &PCG_INC_LO.to_string()),
        abi::move_immediate(&inc_hi, "Integer", &PCG_INC_HI.to_string()),
        abi::add_carry(lo, &carry, &prod_lo, &inc_lo, abi::ZERO),
        abi::add_carry(hi, abi::ZERO, &prod_hi, &inc_hi, &carry),
    ]);
}

/// `_mfb_rng_next` — advance the calling thread's PCG64 generator one step and
/// return the next 64-bit value in `x0`. State lives in the arena (`x19`).
pub(super) fn lower_rng_next() -> CodeFunction {
    emit_rng_draw(
        "runtime.rng_next",
        RNG_NEXT_SYMBOL,
        ARENA_RNG_STATE_LO_OFFSET,
        ARENA_RNG_STATE_HI_OFFSET,
    )
}

/// One PCG64 draw over vregs: load state from `[x19 + lo/hi]`, step it, store it
/// back, and return the XSL-RR output (rotate `hi ^ lo` right by the top 6 bits
/// of `hi`) in `x0`. Shared by the main RNG (`rng_next`) and the per-arena fill
/// RNG (`arena_fill_next`). `x19` is the reserved arena register (`arena_base`).
fn emit_rng_draw(name: &str, symbol: &str, lo_offset: usize, hi_offset: usize) -> CodeFunction {
    let mut vregs = Vregs::new();
    let lo = vregs.next();
    let hi = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64(&lo, ARENA_STATE_REGISTER, lo_offset),
        abi::load_u64(&hi, ARENA_STATE_REGISTER, hi_offset),
    ];
    emit_pcg_step(&mut instructions, &mut vregs, &lo, &hi);
    let shift = vregs.next();
    let xored = vregs.next();
    instructions.extend([
        abi::store_u64(&lo, ARENA_STATE_REGISTER, lo_offset),
        abi::store_u64(&hi, ARENA_STATE_REGISTER, hi_offset),
        abi::shift_right_immediate(&shift, &hi, 58),
        abi::exclusive_or_registers(&xored, &hi, &lo),
        abi::rotate_right_registers(abi::return_register(), &xored, &shift),
        abi::return_(),
    ]);
    finalize_vreg_helper(name, symbol, "Integer", instructions, Vec::new())
}

/// `_mfb_rng_seed_at(x0 = arena ptr, x1 = seed)` — initialize the PCG64 state at
/// the given arena from a 64-bit seed, following the canonical seeding dance
/// (`state = 0; step; state += seed; step`).
pub(super) fn lower_rng_seed_at() -> CodeFunction {
    emit_seed_dance(
        "runtime.rng_seed_at",
        RNG_SEED_SYMBOL,
        ARENA_RNG_STATE_LO_OFFSET,
        ARENA_RNG_STATE_HI_OFFSET,
    )
}

/// The canonical PCG64 seeding dance over vregs: `state = 0; step; state +=
/// seed(x1); step; store at x0+lo/hi`. Shared by the main RNG (`rng_seed_at`)
/// and the per-arena fill RNG (`arena_fill_seed`) — same dance, different state
/// words. `x0` (arena ptr) and `x1` (seed) stay physical ABI registers (they are
/// not in the allocatable set, so the allocator never colors them).
fn emit_seed_dance(name: &str, symbol: &str, lo_offset: usize, hi_offset: usize) -> CodeFunction {
    let mut vregs = Vregs::new();
    // Copy the `x0` (arena ptr) and `x1` (seed) ABI args into vregs that survive
    // the `emit_pcg_step` `mul`/`umulh` — on x86 those clobber the registers
    // `x0`/`x1` map to (`rax`/`rdx`), which would otherwise destroy the seed and
    // the store base mid-dance.
    let ptr = vregs.next();
    let seed = vregs.next();
    let lo = vregs.next();
    let hi = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&ptr, abi::ARG[0]),
        abi::move_register(&seed, abi::ARG[1]),
        abi::move_immediate(&lo, "Integer", "0"),
        abi::move_immediate(&hi, "Integer", "0"),
    ];
    emit_pcg_step(&mut instructions, &mut vregs, &lo, &hi);
    let carry = vregs.next();
    instructions.extend([
        // state += seed, carry as an explicit value (plan-00-G §4).
        abi::add_carry(&lo, &carry, &lo, &seed, abi::ZERO),
        abi::add_carry(&hi, abi::ZERO, &hi, abi::ZERO, &carry),
    ]);
    emit_pcg_step(&mut instructions, &mut vregs, &lo, &hi);
    instructions.extend([
        abi::store_u64(&lo, &ptr, lo_offset),
        abi::store_u64(&hi, &ptr, hi_offset),
        abi::return_(),
    ]);
    finalize_vreg_helper(name, symbol, "Nothing", instructions, Vec::new())
}

/// `arena_fill_seed(x0 = arena ptr, x1 = seed)` — seed the dedicated fill RNG at
/// offsets 16/24 from a 64-bit seed (same PCG64 dance as `rng_seed_at`, different
/// state words). Leaf; clobbers x9–x16.
pub(super) fn lower_arena_fill_seed() -> CodeFunction {
    emit_seed_dance(
        "runtime.arena_fill_seed",
        ARENA_FILL_SEED_SYMBOL,
        ARENA_FILL_RNG_LO_OFFSET,
        ARENA_FILL_RNG_HI_OFFSET,
    )
}

/// `arena_fill_next()` — advance the calling thread's fill RNG (`x19`, offsets
/// 16/24) and return the next 64-bit XSL-RR output in `x0`. Leaf; clobbers
/// x9–x16. Used only to draw a child fill seed from the parent at spawn.
pub(super) fn lower_arena_fill_next() -> CodeFunction {
    emit_rng_draw(
        "runtime.arena_fill_next",
        ARENA_FILL_NEXT_SYMBOL,
        ARENA_FILL_RNG_LO_OFFSET,
        ARENA_FILL_RNG_HI_OFFSET,
    )
}

/// `arena_fill_random(x0 = ptr, x1 = len)` — overwrite `len` bytes at `ptr` with
/// output from the calling thread's fill RNG. `len` is rounded up to an 8-byte
/// word; every chunk handed to this helper is a multiple of 16 bytes, so the
/// rounding is exact and never writes past the chunk. Streams PRNG words without
/// a syscall (§6.1). Leaf; clobbers x0, x1, x9–x16.
pub(super) fn lower_arena_fill_random() -> CodeFunction {
    let mut vregs = Vregs::new();
    // The PCG64 state is loop-carried across the fill loop, so `lo`/`hi` are the
    // same vregs the allocator keeps in registers across the back-edge. The `x0`
    // (ptr) and `x1` (word count) ABI args become loop-carried vregs too: copy
    // them in at entry so the allocator places them in callee-saved registers.
    // On AArch64 they could stay physical (a leaf clobbers nothing), but x86's
    // `mul`/`umulh` in the PCG step clobber the registers `x0`/`x1` map to
    // (`rax`/`rdx`), so a physical counter would be destroyed mid-loop.
    let ptr = vregs.next();
    let count = vregs.next();
    let lo = vregs.next();
    let hi = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&ptr, abi::ARG[0]),
        abi::move_register(&count, abi::ARG[1]),
        // word count = (len + 7) >> 3
        abi::add_immediate(&count, &count, 7),
        abi::shift_right_immediate(&count, &count, 3),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq("arena_fill_done"),
        abi::load_u64(&lo, ARENA_STATE_REGISTER, ARENA_FILL_RNG_LO_OFFSET),
        abi::load_u64(&hi, ARENA_STATE_REGISTER, ARENA_FILL_RNG_HI_OFFSET),
        abi::label("arena_fill_loop"),
    ];
    emit_pcg_step(&mut instructions, &mut vregs, &lo, &hi);
    let shift = vregs.next();
    let xored = vregs.next();
    let word = vregs.next();
    instructions.extend([
        abi::shift_right_immediate(&shift, &hi, 58),
        abi::exclusive_or_registers(&xored, &hi, &lo),
        abi::rotate_right_registers(&word, &xored, &shift),
        abi::store_u64(&word, &ptr, 0),
        abi::add_immediate(&ptr, &ptr, 8),
        abi::subtract_immediate(&count, &count, 1),
        abi::compare_immediate(&count, "0"),
        abi::branch_ne("arena_fill_loop"),
        abi::store_u64(&lo, ARENA_STATE_REGISTER, ARENA_FILL_RNG_LO_OFFSET),
        abi::store_u64(&hi, ARENA_STATE_REGISTER, ARENA_FILL_RNG_HI_OFFSET),
        abi::label("arena_fill_done"),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.arena_fill_random",
        ARENA_FILL_RANDOM_SYMBOL,
        "Nothing",
        instructions,
        Vec::new(),
    )
}
