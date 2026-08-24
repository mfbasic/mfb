//! ISA-neutral register allocator core (plan-03-register-allocator).
//!
//! Lowerings mint **virtual registers** through `CodeBuilder::allocate_register`
//! instead of naming a physical register. A virtual register is carried in the
//! instruction stream as the sentinel string `%vN` in any register-valued
//! operand field. After a function is fully lowered, [`allocate`] rewrites every
//! `%vN` to a physical register (or a spill slot) by liveness-driven linear
//! scan, driven by the per-ISA
//! [`RegisterModel`](crate::target::shared::regmodel::RegisterModel).
//!
//! Linear scan is the only allocation method. The legacy `BumpAndReset`
//! replay (the `--regalloc bump` differential oracle, plan-03 Stage A) was
//! removed: it had no spilling, so under register pressure it reused a
//! still-live register and miscompiled — it was never a correctness baseline,
//! and the `--regalloc` flag went with it.

use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::target::shared::regmodel::{RegClass, RegisterModel};

mod analysis;
use analysis::ClassModel;
pub(crate) mod builder_registers;
mod linear_scan;

/// The sentinel prefix an integer virtual register carries in an instruction
/// field. It cannot collide with any physical register name, immediate, symbol,
/// label, or type name (none of which begin with `%`).
const VREG_PREFIX: &str = "%v";

/// The sentinel prefix a floating-point virtual register carries (plan-03 Stage
/// C). Distinct from the integer prefix so the two classes are allocated
/// independently.
const FP_VREG_PREFIX: &str = "%f";

/// Render integer virtual register index `n` as its instruction-field sentinel.
pub(crate) fn vreg_name(n: u32) -> String {
    format!("{VREG_PREFIX}{n}")
}

/// Parse an integer virtual-register sentinel back to its index, or `None`.
pub(crate) fn parse_vreg(value: &str) -> Option<u32> {
    value.strip_prefix(VREG_PREFIX)?.parse().ok()
}

/// Render floating-point virtual register index `n` as its sentinel.
pub(crate) fn fp_vreg_name(n: u32) -> String {
    format!("{FP_VREG_PREFIX}{n}")
}

/// Parse a floating-point virtual-register sentinel back to its index, or `None`.
pub(crate) fn parse_fp_vreg(value: &str) -> Option<u32> {
    value.strip_prefix(FP_VREG_PREFIX)?.parse().ok()
}

/// What coloring produced that the caller (`finalize_frame` setup) must apply:
/// the stack-slot offsets allocated for spilled values and the callee-saved
/// registers the coloring newly used.
pub(crate) struct AllocOutcome {
    /// Offsets (pre-prologue, `sp`-relative) of stack slots allocated for spills,
    /// in slot order.
    pub(crate) spill_slots: Vec<usize>,
    /// Callee-saved registers the coloring used that the frame must save.
    pub(crate) extra_callee_saved: Vec<String>,
}

/// plan-34-D stream invariant: a shared-lowering-emitted stream — a function
/// body before selection/allocation, or a machine-floor stream (entry stub,
/// thread trampoline) — names **no physical register**. Scratch is a virtual
/// register or a neutral `abi` token pool, the call boundary is role tokens,
/// pinned registers are tokens, and the stack pointer is the neutral `sp`.
/// Physical names enter a stream only downstream: token realization in
/// `Backend::select` and coloring in [`allocate`].
///
/// Returns a description of the first offending operand, or `None`. `%`-headed
/// values are tokens/vregs by construction (the sentinel prefix cannot collide
/// with a physical name) and are skipped — the occupancy parsers deliberately
/// map the FP scratch tokens to physical indices, so they must not be
/// misreported here.
pub(crate) fn find_physical_operand(instructions: &[CodeInstruction]) -> Option<String> {
    for (index, instruction) in instructions.iter().enumerate() {
        for (name, value) in &instruction.fields {
            // bug-176 D: only register-role operand fields can carry a physical
            // register. The string-reference fields — a branch/call `target` and a
            // `label`'s `name` — hold user/label symbols, so a symbol literally
            // spelled like a register (`ra`, `gp`, `s0`, `w0`, `q3`) must not be
            // misreported as a zero-physical-register regression. Registers never
            // use these field names, so skipping them cannot mask a real leak.
            if matches!(*name, "target" | "name") {
                continue;
            }
            // A virtual-register sentinel (`%vN`/`%fN`) can never name a physical
            // register, so match it directly and skip — the pre-allocation stream
            // this scans is dominated by vregs, and `rendered()` would otherwise
            // format each one to a `%`-string only to discard it at the
            // `starts_with('%')` check below (byte-identical: a vreg renders to
            // `%…` and would take that same skip).
            if matches!(value, Operand::VReg { .. }) {
                continue;
            }
            // plan-78-B: render the typed operand to its string for the
            // physical-name sniff (unchanged classification). `Raw`/`Phys` borrow.
            let value = value.rendered();
            if value.starts_with('%') || value == "sp" {
                continue;
            }
            // The occupancy parsers cover every spelling a stream can carry
            // (x/d/v, x86, riscv); the `w`/`s`/`q` views never appear in
            // streams today, but a conservative guard rejects them too.
            let extra_view = value
                .strip_prefix(['w', 's', 'q'])
                .and_then(|rest| rest.parse::<u32>().ok())
                .is_some_and(|n| n <= 31);
            if extra_view
                || analysis::int_physical_index(&value).is_some()
                || analysis::fp_physical_index(&value).is_some()
            {
                return Some(format!(
                    "instruction {index} `{}` field `{name}` names physical register `{value}`",
                    instruction.op.mnemonic()
                ));
            }
        }
    }
    None
}

/// Color a fully-lowered function and rewrite its virtual registers in place.
///
/// `spill_base_offset` is the current frame size, where any spill slots are
/// placed. Must run before the peephole pass and `finalize_frame` (which
/// expect physical register names).
pub(crate) fn allocate(
    instructions: &mut Vec<CodeInstruction>,
    model: &dyn RegisterModel,
    spill_base_offset: usize,
    reserved: &[&str],
) -> AllocOutcome {
    // plan-78 Phase 1: env-gated size probe. When `MFB_BENCH_LOWERING` is set,
    // report the pre-allocation instruction count and the distinct virtual-
    // register count (int + fp) of each sizable function, so B/C can quote
    // "N instructions / M vregs colored" against a real baseline. Gated on both
    // the env var and a size threshold, it prints nothing on a normal build and
    // only surfaces the large generated bodies (e.g. the inlined regex engine)
    // rather than every tiny helper. It reads only, never mutates the stream, so
    // it cannot affect emitted bytes.
    if std::env::var_os("MFB_BENCH_LOWERING").is_some() && instructions.len() >= 100_000 {
        let mut int_vregs = std::collections::HashSet::new();
        let mut fp_vregs = std::collections::HashSet::new();
        for instruction in instructions.iter() {
            for (_name, value) in &instruction.fields {
                let value = value.rendered();
                if let Some(id) = parse_vreg(&value) {
                    int_vregs.insert(id);
                } else if let Some(id) = parse_fp_vreg(&value) {
                    fp_vregs.insert(id);
                }
            }
        }
        eprintln!(
            "MFB_BENCH_LOWERING: function instructions={} int_vregs={} fp_vregs={}",
            instructions.len(),
            int_vregs.len(),
            fp_vregs.len()
        );
    }
    {
        // Allocate the integer class, then the FP class over the
        // already-integer-colored stream. The two physical files never
        // interfere, so each pass sees only its own operands; FP spill slots
        // are placed after the integer ones.
        // The `%scratch`/`%sysnr` occupancy indices in `int_physical_index` are
        // AArch64 realizations; on x86/riscv those tokens realize elsewhere (and
        // are lowered to concrete names before allocation), so pick the variant
        // that omits the AArch64 scratch arms off-target (bug-127).
        let is_aarch64 = model.arena_base() == crate::arch::aarch64::regmodel::ARENA_BASE_REGISTER;
        let int_physical_index = if is_aarch64 {
            analysis::int_physical_index
        } else {
            analysis::int_physical_index_non_aarch64
        };
        // The call-clobber masks are indexed by physical-register *number*,
        // so they cannot be shared across ISAs (the same bit means `d8` on
        // AArch64 and `xmm8` on x86). Derive each from the target's own
        // caller-saved table rather than restating it as a constant — that
        // restatement is what let x86-64 inherit AArch64's masks and model
        // `xmm8`–`xmm14` as surviving a call SysV says destroys them
        // (bug-350).
        let int_model = ClassModel {
            class: RegClass::Int,
            parse_vreg,
            physical_index: int_physical_index,
            is_fp: false,
            caller_saved: analysis::caller_saved_mask(model, RegClass::Int, int_physical_index),
        };
        let fp_model = ClassModel {
            class: RegClass::Fp,
            parse_vreg: parse_fp_vreg,
            physical_index: analysis::fp_physical_index,
            is_fp: true,
            caller_saved: analysis::caller_saved_mask(
                model,
                RegClass::Fp,
                analysis::fp_physical_index,
            ),
        };
        // Uniform per-slot stride so any class fits (x86 16 for a 128-bit FP
        // spill; AArch64 8 — a no-op, byte-identical).
        let slot_bytes = model.spill_slot_bytes();
        let int = linear_scan::run(
            std::mem::take(instructions),
            model,
            RegClass::Int,
            &int_model,
            spill_base_offset,
            slot_bytes,
            reserved,
        );
        // No valid register allocation exists (bug-127.2): an instruction names
        // more simultaneously-live registers than the target's integer pool
        // holds. This is a codegen defect (an ISA `select` emitting an
        // over-wide instruction, or a mis-sized pool), not user input, so it is
        // an ICE — but a clear, actionable one surfaced at the allocation
        // boundary rather than the raw operand-count `.expect` it replaced. A
        // user-facing diagnostic would require threading a `Result` out through
        // `allocate` and its callers.
        if let Some(error) = int.error {
            panic!("{error}");
        }
        *instructions = int.instructions;
        let fp_base = spill_base_offset + int.spill_slot_count * slot_bytes;
        let fp = linear_scan::run(
            std::mem::take(instructions),
            model,
            RegClass::Fp,
            &fp_model,
            fp_base,
            slot_bytes,
            reserved,
        );
        if let Some(error) = fp.error {
            panic!("{error}");
        }
        *instructions = fp.instructions;

        // Fail-safe (bug-242): liveness sees register operands only through the
        // hardcoded DEF_FIELDS/USE_FIELDS allowlist, so a future register-valued
        // field name not listed there would be invisible to allocation and left
        // as a raw `%v`/`%f` sentinel. Assert none survives, so an uncovered
        // field fails loudly here in debug builds instead of silently emitting a
        // bogus operand.
        debug_assert!(
            !instructions
                .iter()
                .any(|instruction| instruction.fields.iter().any(|(_, value)| {
                    let value = value.rendered();
                    parse_vreg(&value).is_some() || parse_fp_vreg(&value).is_some()
                })),
            "regalloc left an uncolored vreg/fp-vreg sentinel in an operand field \
                 (a register-valued field not covered by DEF_FIELDS/USE_FIELDS?)"
        );

        let total_spills = int.spill_slot_count + fp.spill_slot_count;
        let spill_slots = (0..total_spills)
            .map(|k| spill_base_offset + k * slot_bytes)
            .collect();
        let mut extra_callee_saved = int.extra_callee_saved;
        for register in fp.extra_callee_saved {
            if !extra_callee_saved.contains(&register) {
                extra_callee_saved.push(register);
            }
        }
        AllocOutcome {
            spill_slots,
            extra_callee_saved,
        }
    }
}

/// Thin wrappers exposing integer liveness to the FP-shuttle peephole
/// (`crate::optimizer::opt2::peephole`), which proves a GPR carrying only a float's bit pattern is
/// dead before dropping the shuttle. (The analysis items are `pub(crate)` within
/// `regalloc`, so they are surfaced to the parent module through these wrappers
/// rather than re-exported.)
pub(crate) fn integer_live_out(
    instructions: &[CodeInstruction],
    model: &dyn RegisterModel,
) -> Vec<u64> {
    analysis::integer_live_out(instructions, model)
}

pub(crate) fn physical_busy(bits: u64, index: u32) -> bool {
    analysis::physical_busy(bits, index)
}

pub(crate) fn int_physical_index(name: &str) -> Option<u32> {
    analysis::int_physical_index(name)
}

#[cfg(test)]
mod tests;
