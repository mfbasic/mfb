//! ISA-neutral register allocator core (plan-03-register-allocator).
//!
//! Lowerings mint **virtual registers** through `CodeBuilder::allocate_register`
//! instead of naming a physical register. A virtual register is carried in the
//! instruction stream as the sentinel string `%vN` in any register-valued
//! operand field. After a function is fully lowered, [`allocate`] rewrites every
//! `%vN` to a physical register (or a spill slot), driven by a pluggable
//! [`AllocationStrategy`] and the per-ISA
//! [`RegisterModel`](crate::target::shared::regmodel::RegisterModel).
//!
//! The strategy is selected by the `-regalloc <name>` build flag (§4.2). Stage A
//! ships exactly one strategy, [`BumpAndReset`], which reproduces the legacy
//! bump-and-reset physical assignment byte-for-byte — it is the reference /
//! differential-debugging oracle the later liveness-driven strategies validate
//! against.

use std::sync::OnceLock;

use crate::target::shared::regmodel::{RegClass, RegisterModel};

use super::types::CodeInstruction;
use analysis::ClassModel;

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

/// Which allocation method to run. Selected by `-regalloc <name>`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RegallocKind {
    /// Replay the legacy per-statement bump numbering. Byte-identical to the
    /// pre-allocator backend; kept permanently as the `-regalloc bump` oracle.
    ///
    /// This is a debugging/differential reference, **not** a correct allocator:
    /// it has no spilling, so on high register pressure it reuses a still-live
    /// register and miscompiles — exactly the legacy bug class [`LinearScan`] was
    /// built to fix. Known divergences where bump is the wrong one (and
    /// linear-scan correct) are `tests/rt-behavior/control-flow/control-flow-behavior` (a value clobbered across a
    /// call) and the `float-nbody` benchmark (the advance loop's float pressure).
    /// Never default to it or treat its output as a correctness baseline.
    BumpAndReset,
    /// Liveness-driven linear-scan over the integer class with spilling
    /// (plan-03 Stage B).
    LinearScan,
}

impl RegallocKind {
    #[allow(dead_code)]
    pub(crate) fn name(self) -> &'static str {
        match self {
            RegallocKind::BumpAndReset => "bump",
            RegallocKind::LinearScan => "linear-scan",
        }
    }
}

/// Names accepted by `-regalloc`, for the error message on an unknown value.
pub(crate) fn available_strategies() -> &'static [&'static str] {
    &["bump", "linear-scan"]
}

/// Parse a `-regalloc` value, listing the available strategies on an unknown
/// name.
pub(crate) fn parse_kind(value: &str) -> Result<RegallocKind, String> {
    match value {
        "bump" => Ok(RegallocKind::BumpAndReset),
        "linear-scan" => Ok(RegallocKind::LinearScan),
        other => Err(format!(
            "unknown -regalloc strategy `{other}` (available: {})",
            available_strategies().join(", ")
        )),
    }
}

static SELECTED: OnceLock<RegallocKind> = OnceLock::new();

/// Record the process-wide allocation strategy chosen on the command line. May
/// be called at most once per process; ignored if already set.
pub(crate) fn set_strategy(kind: RegallocKind) {
    let _ = SELECTED.set(kind);
}

/// The active allocation strategy, defaulting to [`RegallocKind::LinearScan`]
/// (the liveness-driven allocator with spilling, plan-03 Stage B). `bump` remains
/// available as the byte-identical reference oracle via `-regalloc bump`.
pub(crate) fn active_kind() -> RegallocKind {
    *SELECTED.get().unwrap_or(&RegallocKind::LinearScan)
}

/// The inputs an [`AllocationStrategy`] consumes to color a function.
pub(crate) struct AllocInput<'a> {
    /// The fully-lowered instruction stream (virtual registers still present).
    /// Read by the liveness-driven strategies (Stage B); the bump reference
    /// strategy ignores it.
    #[allow(dead_code)]
    pub(crate) instructions: &'a [CodeInstruction],
    /// Per-virtual-register physical assignment the bump allocator computed
    /// eagerly during lowering (index == virtual register number). The
    /// [`BumpAndReset`] reference strategy returns this verbatim.
    pub(crate) eager: &'a [String],
    /// The target register description (§5). Queried by the liveness-driven
    /// strategies (Stage B); the bump reference strategy ignores it.
    #[allow(dead_code)]
    pub(crate) model: &'a dyn RegisterModel,
}

/// A strategy's coloring result.
pub(crate) struct Allocation {
    /// Virtual register index -> physical register name.
    pub(crate) physical: Vec<String>,
    /// Callee-saved registers the strategy used that the frame must save. Empty
    /// for [`BumpAndReset`] (which marks them eagerly during lowering, matching
    /// the legacy save order).
    pub(crate) extra_callee_saved: Vec<String>,
}

/// A swappable register-allocation method (§4.2). Liveness and the rewrite are
/// shared infrastructure; only the assignment policy lives behind this trait, so
/// linear-scan / graph-coloring slot in without touching the rest of the
/// backend.
pub(crate) trait AllocationStrategy {
    fn assign(&self, input: &AllocInput<'_>) -> Allocation;
}

/// The reference strategy: replay the legacy bump-and-reset assignment from the
/// per-virtual-register physical the lowering computed eagerly. Byte-identical
/// to the pre-allocator backend.
pub(crate) struct BumpAndReset;

impl AllocationStrategy for BumpAndReset {
    fn assign(&self, input: &AllocInput<'_>) -> Allocation {
        Allocation {
            physical: input.eager.to_vec(),
            extra_callee_saved: Vec::new(),
        }
    }
}

/// What coloring produced that the caller (`finalize_frame` setup) must apply:
/// the stack-slot offsets allocated for spilled values and the callee-saved
/// registers the coloring newly used.
pub(crate) struct AllocOutcome {
    /// Offsets (pre-prologue, `sp`-relative) of stack slots allocated for spills,
    /// in slot order. Empty for [`RegallocKind::BumpAndReset`].
    pub(crate) spill_slots: Vec<usize>,
    /// Callee-saved registers the coloring used that the frame must save. Empty
    /// for `BumpAndReset` (it marks them during lowering).
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
                || analysis::int_physical_index(value).is_some()
                || analysis::fp_physical_index(value).is_some()
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
/// `eager` holds the bump allocator's per-virtual-register physical (index ==
/// virtual register number), used by `BumpAndReset`. `spill_base_offset` is the
/// current frame size, where any spill slots are placed. Must run before the
/// peephole pass and `finalize_frame` (which expect physical register names).
pub(crate) fn allocate(
    kind: RegallocKind,
    instructions: &mut Vec<CodeInstruction>,
    eager: &[String],
    fp_eager: &[String],
    model: &dyn RegisterModel,
    spill_base_offset: usize,
    reserved: &[&str],
) -> AllocOutcome {
    match kind {
        RegallocKind::BumpAndReset => {
            let allocation = BumpAndReset.assign(&AllocInput {
                instructions,
                eager,
                model,
            });
            rewrite(instructions, parse_vreg, &allocation.physical);
            rewrite(instructions, parse_fp_vreg, fp_eager);
            AllocOutcome {
                spill_slots: Vec::new(),
                extra_callee_saved: allocation.extra_callee_saved,
            }
        }
        RegallocKind::LinearScan => {
            // Allocate the integer class, then the FP class over the
            // already-integer-colored stream. The two physical files never
            // interfere, so each pass sees only its own operands; FP spill slots
            // are placed after the integer ones.
            // rv64's caller-saved set lives at different physical indices than
            // AArch64/x86, so the call-clobber masks are ISA-specific (plan-99).
            // The arena-base register identifies the ISA (`s11` on rv64).
            let is_riscv = model.arena_base() == crate::arch::riscv64::regmodel::ARENA_BASE_REGISTER;
            // The `%scratch`/`%sysnr` occupancy indices in `int_physical_index` are
            // AArch64 realizations; on x86/riscv those tokens realize elsewhere (and
            // are lowered to concrete names before allocation), so pick the variant
            // that omits the AArch64 scratch arms off-target (bug-127).
            let is_aarch64 =
                model.arena_base() == crate::arch::aarch64::regmodel::ARENA_BASE_REGISTER;
            let int_physical_index = if is_aarch64 {
                analysis::int_physical_index
            } else {
                analysis::int_physical_index_non_aarch64
            };
            let int_model = ClassModel {
                parse_vreg,
                physical_index: int_physical_index,
                is_fp: false,
                is_riscv,
            };
            let fp_model = ClassModel {
                parse_vreg: parse_fp_vreg,
                physical_index: analysis::fp_physical_index,
                is_fp: true,
                is_riscv,
            };
            // Uniform per-slot stride so any class fits (x86 16 for a 128-bit FP
            // spill; AArch64 8 — a no-op, byte-identical).
            let slot_bytes = model.spill_slot_bytes();
            let int = linear_scan::run(
                instructions,
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
                instructions,
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
}

/// Substitute every virtual-register sentinel matched by `parse` with its
/// assigned physical register (the `BumpAndReset` rewrite, run once per class).
fn rewrite(
    instructions: &mut [CodeInstruction],
    parse: fn(&str) -> Option<u32>,
    physical: &[String],
) {
    for instruction in instructions.iter_mut() {
        for (_name, value) in instruction.fields.iter_mut() {
            if let Some(index) = parse(value) {
                let assigned = physical.get(index as usize).unwrap_or_else(|| {
                    panic!("register allocator: virtual register {index} has no assignment")
                });
                *value = assigned.clone();
            }
        }
    }
}

mod analysis;
mod linear_scan;

/// Thin wrappers exposing integer liveness to the FP-shuttle peephole
/// (`super::peephole`), which proves a GPR carrying only a float's bit pattern is
/// dead before dropping the shuttle. (The analysis items are `pub(super)` within
/// `regalloc`, so they are surfaced to the parent module through these wrappers
/// rather than re-exported.)
pub(super) fn integer_live_out(instructions: &[CodeInstruction], is_riscv: bool) -> Vec<u64> {
    analysis::integer_live_out(instructions, is_riscv)
}

pub(super) fn physical_busy(bits: u64, index: u32) -> bool {
    analysis::physical_busy(bits, index)
}

pub(super) fn int_physical_index(name: &str) -> Option<u32> {
    analysis::int_physical_index(name)
}

#[cfg(test)]
mod tests;
