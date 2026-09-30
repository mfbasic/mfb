// --- codegen tier imports (migration) ---
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
/// One PCG64 step `state = state * MULT + INC` (128-bit), over virtual registers.
/// `lo`/`hi` are the caller's state vregs (read and rewritten in place); all
/// scratch is freshly generated. The increment is added with explicit-carry
/// `add_carry` (the carry is a vreg value, not the flags register — plan-00-G §4),
/// so the chain survives register allocation.
fn emit_pcg_step(
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
    lo: impl Into<Reg>,
    hi: impl Into<Reg>,
) {
    let (lo, hi): (Reg, Reg) = (lo.into(), hi.into());
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
pub(crate) fn lower_rng_next() -> CodeFunction {
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
pub(crate) fn lower_rng_seed_at() -> CodeFunction {
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
        abi::move_register(&ptr, abi::c_arg(0)),
        abi::move_register(&seed, abi::c_arg(1)),
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
pub(crate) fn lower_arena_fill_seed() -> CodeFunction {
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
pub(crate) fn lower_arena_fill_next() -> CodeFunction {
    emit_rng_draw(
        "runtime.arena_fill_next",
        ARENA_FILL_NEXT_SYMBOL,
        ARENA_FILL_RNG_LO_OFFSET,
        ARENA_FILL_RNG_HI_OFFSET,
    )
}

/// Number of fill words one iteration of `_mfb_arena_fill_random`'s main loop
/// writes (bug-769 G2). Each of the `FILL_UNROLL` words is its own 128-bit
/// multiply-add from the same loop-carried state (a PCG64 jump-ahead), so the
/// words do not wait on one another's multiply the way consecutive single steps
/// do; the stream is exactly the single-step stream. 4 measured fastest on macOS
/// aarch64 and on a real Coffee Lake x86-64 (Win64): ahead of 2, level with 8.
const FILL_UNROLL: usize = 4;

/// The PCG64 LCG multiplier and increment as 128-bit values.
const PCG_MULT: u128 = ((PCG_MULT_HI as u128) << 64) | PCG_MULT_LO as u128;
const PCG_INC: u128 = ((PCG_INC_HI as u128) << 64) | PCG_INC_LO as u128;

/// The affine map of `steps` PCG64 steps: `state' = state * mult + inc` with
/// `mult = M^steps` and `inc = I * (M^(steps-1) + … + M + 1)`, wrapping mod
/// 2^128. Composing one more step onto `(m, c)` gives `(m * M, c * M + I)`.
const fn pcg_jump(steps: usize) -> (u128, u128) {
    let mut mult: u128 = 1;
    let mut inc: u128 = 0;
    let mut i = 0;
    while i < steps {
        mult = mult.wrapping_mul(PCG_MULT);
        inc = inc.wrapping_mul(PCG_MULT).wrapping_add(PCG_INC);
        i += 1;
    }
    (mult, inc)
}

/// `pcg_jump(1..=FILL_UNROLL)`, evaluated at compile time: entry `j - 1` maps the
/// loop-carried state to the state `j` steps on.
const FILL_JUMPS: [(u128, u128); FILL_UNROLL] = {
    let mut table = [(0u128, 0u128); FILL_UNROLL];
    let mut j = 0;
    while j < FILL_UNROLL {
        table[j] = pcg_jump(j + 1);
        j += 1;
    }
    table
};

/// `arena_fill_random(x0 = ptr, x1 = len)` — overwrite `len` bytes at `ptr` with
/// output from the calling thread's fill RNG. `len` is rounded up to an 8-byte
/// word; every chunk handed to this helper is a multiple of 16 bytes, so the
/// rounding is exact and never writes past the chunk. Streams PRNG words without
/// a syscall (§6.1). Leaf (makes no call); vreg-allocated, so it clobbers
/// caller-saved registers only.
///
/// bug-769 G2: while at least `FILL_UNROLL` words remain, one iteration writes
/// `FILL_UNROLL` words. Word `j` is the XSL-RR output of `FILL_JUMPS[j - 1]`
/// applied to the loop-carried state, so the words' 128-bit multiplies are
/// independent of one another; the last is the state `FILL_UNROLL` steps on and
/// is carried. The remaining `count mod FILL_UNROLL` words take the single step.
/// The words, their order, and the state written back to offsets 16/24 are
/// exactly those of `count` single steps. Each loop's constants are materialized
/// once per call, before the loop; the allocator parks the ones that do not fit
/// a register in a stack slot (one load per use instead of a four-instruction
/// rebuild).
pub(crate) fn lower_arena_fill_random() -> CodeFunction {
    let mut vregs = Vregs::new();
    // The PCG64 state is loop-carried across the fill loops, so `lo`/`hi` are the
    // same vregs the allocator keeps in registers across the back-edges. The `x0`
    // (ptr) and `x1` (word count) ABI args become loop-carried vregs too: copy
    // them in at entry so the allocator places them in callee-saved registers.
    // On AArch64 they could stay physical (a leaf clobbers nothing), but x86's
    // `mul`/`umulh` clobber the registers `x0`/`x1` map to (`rax`/`rdx`), so a
    // physical counter would be destroyed mid-loop.
    let ptr: Reg = vregs.next().into();
    let count: Reg = vregs.next().into();
    let lo: Reg = vregs.next().into();
    let hi: Reg = vregs.next().into();
    let unroll = FILL_UNROLL.to_string();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(ptr, abi::c_arg(0)),
        abi::move_register(count, abi::c_arg(1)),
        // word count = (len + 7) >> 3
        abi::add_immediate(count, count, 7),
        abi::shift_right_immediate(count, count, 3),
        abi::compare_immediate(count, "0"),
        abi::branch_eq("arena_fill_done"),
        abi::load_u64(lo, ARENA_STATE_REGISTER, ARENA_FILL_RNG_LO_OFFSET),
        abi::load_u64(hi, ARENA_STATE_REGISTER, ARENA_FILL_RNG_HI_OFFSET),
        abi::compare_immediate(count, &unroll),
        abi::branch_lt("arena_fill_tail"),
    ];
    let jumps =
        FILL_JUMPS.map(|(mult, inc)| materialize_affine(&mut instructions, &mut vregs, mult, inc));
    instructions.push(abi::label("arena_fill_block"));
    for (index, constants) in jumps.iter().enumerate() {
        // The last jump is the state `FILL_UNROLL` steps on: it is carried, so it
        // lands in `lo`/`hi` (every read of the old state precedes its write).
        let next: [Reg; 2] = if index + 1 == FILL_UNROLL {
            [lo, hi]
        } else {
            [vregs.next().into(), vregs.next().into()]
        };
        emit_affine_step(&mut instructions, &mut vregs, [lo, hi], next, constants);
        emit_xsl_rr_store(&mut instructions, &mut vregs, next, ptr, 8 * index);
    }
    instructions.extend([
        abi::add_immediate(ptr, ptr, 8 * FILL_UNROLL),
        abi::subtract_immediate(count, count, FILL_UNROLL),
        abi::compare_immediate(count, &unroll),
        abi::branch_ge("arena_fill_block"),
        abi::compare_immediate(count, "0"),
        abi::branch_eq("arena_fill_store"),
        abi::label("arena_fill_tail"),
    ]);
    // The tail: one single step per word.
    let single = materialize_affine(&mut instructions, &mut vregs, PCG_MULT, PCG_INC);
    instructions.push(abi::label("arena_fill_loop"));
    emit_affine_step(&mut instructions, &mut vregs, [lo, hi], [lo, hi], &single);
    emit_xsl_rr_store(&mut instructions, &mut vregs, [lo, hi], ptr, 0);
    instructions.extend([
        abi::add_immediate(ptr, ptr, 8),
        abi::subtract_immediate(count, count, 1),
        abi::compare_immediate(count, "0"),
        abi::branch_ne("arena_fill_loop"),
        abi::label("arena_fill_store"),
        abi::store_u64(lo, ARENA_STATE_REGISTER, ARENA_FILL_RNG_LO_OFFSET),
        abi::store_u64(hi, ARENA_STATE_REGISTER, ARENA_FILL_RNG_HI_OFFSET),
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

/// Materialize an affine map's 128-bit multiplier and increment into four fresh
/// vregs, returned as `[mult_lo, mult_hi, inc_lo, inc_hi]`.
fn materialize_affine(
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
    mult: u128,
    inc: u128,
) -> [Reg; 4] {
    [
        mult as u64,
        (mult >> 64) as u64,
        inc as u64,
        (inc >> 64) as u64,
    ]
    .map(|limb| {
        let register: Reg = vregs.next().into();
        instructions.push(abi::move_immediate(register, "Integer", &limb.to_string()));
        register
    })
}

/// `dst = src * mult + inc` over 128-bit `[lo, hi]` limb pairs (wrapping), the
/// constants in vregs as `[mult_lo, mult_hi, inc_lo, inc_hi]`. The arithmetic of
/// `emit_pcg_step`, with the constants supplied rather than rebuilt; `dst` may be
/// `src` (every read of `src` precedes the first write of `dst`).
fn emit_affine_step(
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
    [src_lo, src_hi]: [Reg; 2],
    [dst_lo, dst_hi]: [Reg; 2],
    &[mult_lo, mult_hi, inc_lo, inc_hi]: &[Reg; 4],
) {
    let prod_lo: Reg = vregs.next().into();
    let prod_hi: Reg = vregs.next().into();
    let cross_lo: Reg = vregs.next().into();
    let cross_hi: Reg = vregs.next().into();
    let carry: Reg = vregs.next().into();
    instructions.extend([
        abi::multiply_registers(prod_lo, mult_lo, src_lo),
        abi::unsigned_multiply_high_registers(prod_hi, mult_lo, src_lo),
        abi::multiply_registers(cross_lo, mult_lo, src_hi),
        abi::multiply_registers(cross_hi, mult_hi, src_lo),
        abi::add_registers(prod_hi, prod_hi, cross_lo),
        abi::add_registers(prod_hi, prod_hi, cross_hi),
        abi::add_carry(dst_lo, carry, prod_lo, inc_lo, abi::ZERO),
        abi::add_carry(dst_hi, abi::ZERO, prod_hi, inc_hi, carry),
    ]);
}

/// Store the XSL-RR output of state `[lo, hi]` — `hi ^ lo` rotated right by the
/// top 6 bits of `hi` — at `[base + offset]`.
fn emit_xsl_rr_store(
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
    [lo, hi]: [Reg; 2],
    base: Reg,
    offset: usize,
) {
    let shift: Reg = vregs.next().into();
    let xored: Reg = vregs.next().into();
    let word: Reg = vregs.next().into();
    instructions.extend([
        abi::shift_right_immediate(shift, hi, 58),
        abi::exclusive_or_registers(xored, hi, lo),
        abi::rotate_right_registers(word, xored, shift),
        abi::store_u64(word, base, offset),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One PCG64 step (`emit_pcg_step`) and the XSL-RR output (`emit_rng_draw`).
    fn step(state: u128) -> u128 {
        state.wrapping_mul(PCG_MULT).wrapping_add(PCG_INC)
    }

    fn xsl_rr(state: u128) -> u64 {
        let (hi, lo) = ((state >> 64) as u64, state as u64);
        (hi ^ lo).rotate_right((hi >> 58) as u32)
    }

    fn word_count(len: usize) -> usize {
        (len + 7) >> 3
    }

    /// The pre-G2 helper: one single step per word.
    fn fill_single_step(state: u128, len: usize) -> (Vec<u64>, u128) {
        let mut state = state;
        let mut words = Vec::new();
        for _ in 0..word_count(len) {
            state = step(state);
            words.push(xsl_rr(state));
        }
        (words, state)
    }

    /// `lower_arena_fill_random` as emitted, branch for branch: the entry
    /// `count < FILL_UNROLL` test, the jump-ahead block loop off `FILL_JUMPS`
    /// (word `j` at `ptr + 8j`, the last jump carried), then the single-step tail
    /// through the same affine form with `(PCG_MULT, PCG_INC)`.
    fn fill_unrolled(state: u128, len: usize) -> (Vec<u64>, u128) {
        let affine = |s: u128, (mult, inc): (u128, u128)| s.wrapping_mul(mult).wrapping_add(inc);
        let mut state = state;
        let mut count = word_count(len);
        let mut memory = vec![0u64; count];
        let mut ptr = 0;
        if count == 0 {
            return (memory, state);
        }
        if count >= FILL_UNROLL {
            loop {
                let carried = state;
                for (index, &jump) in FILL_JUMPS.iter().enumerate() {
                    let next = affine(carried, jump);
                    memory[ptr + index] = xsl_rr(next);
                    if index + 1 == FILL_UNROLL {
                        state = next;
                    }
                }
                ptr += FILL_UNROLL;
                count -= FILL_UNROLL;
                if count < FILL_UNROLL {
                    break;
                }
            }
        }
        while count != 0 {
            state = affine(state, (PCG_MULT, PCG_INC));
            memory[ptr] = xsl_rr(state);
            ptr += 1;
            count -= 1;
        }
        (memory, state)
    }

    const SEEDS: [u128; 5] = [
        0,
        1,
        u128::MAX,
        0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210,
        0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645,
    ];

    #[test]
    fn pcg_jump_constants_are_powers_and_geometric_increments() {
        assert_eq!(PCG_MULT, 0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645);
        assert_eq!(PCG_INC, 0x5851_F42D_4C95_7F2D_1405_7B7E_F767_814F);
        for (index, &(mult, inc)) in FILL_JUMPS.iter().enumerate() {
            let j = index as u32 + 1;
            let mut power = 1u128;
            let mut geometric = 0u128;
            for i in 0..j {
                geometric = geometric.wrapping_add(PCG_MULT.wrapping_pow(i));
                power = power.wrapping_mul(PCG_MULT);
            }
            assert_eq!(mult, PCG_MULT.wrapping_pow(j), "M^{j}");
            assert_eq!(mult, power, "M^{j}");
            assert_eq!(inc, PCG_INC.wrapping_mul(geometric), "I * sum M^i, i < {j}");
            // The jump is exactly j single steps from any state.
            for &seed in &SEEDS {
                let stepped = (0..j).fold(seed, |s, _| step(s));
                assert_eq!(seed.wrapping_mul(mult).wrapping_add(inc), stepped);
            }
        }
    }

    #[test]
    fn unrolled_fill_emits_the_single_step_stream() {
        for &seed in &SEEDS {
            for len in (0..=64).chain([4096 * 8, 4096 * 8 - 3, 4096 * 8 + 16]) {
                assert_eq!(
                    fill_unrolled(seed, len),
                    fill_single_step(seed, len),
                    "seed {seed:#x}, len {len}"
                );
            }
        }
    }

    #[test]
    fn consecutive_fills_continue_one_stream() {
        // Fills chain through the state written back to offsets 16/24: a run of
        // fills of mixed lengths is one contiguous single-step stream.
        for &seed in &SEEDS {
            let lengths = [16, 48, 0, 8, 112, 4096, 32, 7, 24];
            let total: usize = lengths.iter().map(|&len| word_count(len) * 8).sum();
            let (expected, expected_state) = fill_single_step(seed, total);
            let mut state = seed;
            let mut stream = Vec::new();
            for len in lengths {
                let (words, next) = fill_unrolled(state, len);
                stream.extend(words);
                state = next;
            }
            assert_eq!(stream, expected);
            assert_eq!(state, expected_state);
        }
    }
}
