//! Known-bits lattice over the SSA overlay — the fact base three Level-2
//! Opt2 rows share: known-bits simplification, narrowing / bit-width
//! reduction, and sign/zero extension elimination.
//!
//! For each SSA value it computes two masks: `ones` (bits provably 1) and
//! `zeros` (bits provably 0). A bit set in neither is unknown; a bit set in
//! both is impossible and never produced. The lattice starts fully unknown
//! and is refined by a pessimistic worklist fixpoint over the same dependency
//! edges the constant-propagation row uses, so a value only ever *gains*
//! knowledge from already-known inputs and loop-carried phis simply stay
//! unknown.
//!
//! Transfer functions cover exactly the ops whose bit behavior is
//! target-independent and total — the pure, flag-free whitelist the other
//! rows already trust:
//!
//! - `mov_imm` (Integer): every bit is known.
//! - `mov`: the source's bits.
//! - `and`/`orr`/`eor`: the classic per-bit rules (`and` knows a zero when
//!   *either* side does; `orr` knows a one when either does; `eor` knows a
//!   bit only when both sides know it).
//! - `lsl_imm`/`lsr_imm`: shift the masks, and the vacated bits are provably
//!   zero — the fact the narrowing and extension rows are built on.
//! - `add`/`add_imm`: only the low run of bits both operands know, and only
//!   while no carry can reach past it.
//! - a phi: the intersection of its arguments' knowledge.
//!
//! Everything else contributes nothing (all bits unknown), so mis-modeling
//! can only lose a rewrite. **Trap discipline is structural**, as everywhere
//! else in this seam: the flag-setting checked-arithmetic ops are not in the
//! table at all, so no row built on these facts can ever reason about — or
//! rewrite — an operation that may raise.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::{classify_ref, is_use_field, ClassModel, RegRef};
use crate::codegen::engine::types::CodeInstruction;

use super::ssa::{Ssa, ValueDef};

/// What is provably known about one value's 64 bits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Known {
    /// Bits provably 1.
    pub(crate) ones: u64,
    /// Bits provably 0.
    pub(crate) zeros: u64,
}

impl Known {
    /// Nothing is known.
    pub(crate) const UNKNOWN: Known = Known { ones: 0, zeros: 0 };

    /// Every bit of `value` is known.
    pub(crate) fn exact(value: u64) -> Known {
        Known {
            ones: value,
            zeros: !value,
        }
    }

    /// The meet: only what both sides agree on.
    fn meet(self, other: Known) -> Known {
        Known {
            ones: self.ones & other.ones,
            zeros: self.zeros & other.zeros,
        }
    }

    /// Whether every bit outside `mask` is provably zero — the question the
    /// narrowing and extension rows ask ("does this value fit in 32 bits?",
    /// "is the high half already clear?").
    pub(crate) fn fits_in(&self, mask: u64) -> bool {
        self.zeros & !mask == !mask
    }

    fn is_unknown(&self) -> bool {
        self.ones == 0 && self.zeros == 0
    }
}

/// Known bits per SSA value, indexed by value id.
pub(crate) fn analyze(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
) -> Vec<Known> {
    let mut known = vec![Known::UNKNOWN; overlay.values.len()];

    // Consumer edges, so a refined value re-evaluates exactly its users.
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); overlay.values.len()];
    for (vid, def) in overlay.values.iter().enumerate() {
        match def {
            ValueDef::Inst(i) => {
                for (name, operand) in &instructions[*i].fields {
                    if !is_use_field(name) {
                        continue;
                    }
                    if let Some(RegRef::VReg(id)) = classify_ref(operand, &models.0) {
                        if let Some(w) = overlay.value_of_use(*i, (false, id)) {
                            dependents[w].push(vid);
                        }
                    }
                }
            }
            ValueDef::Phi { args, .. } => {
                for &(_, arg) in args {
                    dependents[arg].push(vid);
                }
            }
            ValueDef::Entry => {}
        }
    }

    let operand_known = |known: &[Known], i: usize, field: &str| -> Known {
        let Some(operand) = instructions[i].operand(field) else {
            return Known::UNKNOWN;
        };
        match classify_ref(operand, &models.0) {
            Some(RegRef::VReg(id)) => match overlay.value_of_use(i, (false, id)) {
                Some(value) => known[value],
                None => Known::UNKNOWN,
            },
            Some(RegRef::Phys(_)) => Known::UNKNOWN,
            None => match super::super::branches::bits(&operand.rendered()) {
                Some(value) => Known::exact(value),
                None => Known::UNKNOWN,
            },
        }
    };

    let mut work: Vec<usize> = (0..overlay.values.len()).collect();
    let mut rounds = 0usize;
    // The lattice has 64 bits × 2 masks per value and only ever gains bits, so
    // it converges; the cap bounds pathological CFGs the same way the memory
    // dataflow's does.
    while let Some(vid) = work.pop() {
        rounds += 1;
        if rounds > overlay.values.len().saturating_mul(8) + 1024 {
            break;
        }
        let next = match &overlay.values[vid] {
            ValueDef::Entry => Known::UNKNOWN,
            ValueDef::Phi { args, .. } => args
                .iter()
                .map(|&(_, arg)| known[arg])
                .reduce(Known::meet)
                .unwrap_or(Known::UNKNOWN),
            ValueDef::Inst(i) => {
                transfer(&instructions[*i], |field| operand_known(&known, *i, field))
            }
        };
        if next == known[vid] {
            continue;
        }
        known[vid] = next;
        work.extend(dependents[vid].iter().copied());
    }
    known
}

/// One instruction's known-bits transfer (the module docs' table).
fn transfer(instruction: &CodeInstruction, operand: impl Fn(&str) -> Known) -> Known {
    match instruction.op {
        CodeOp::MovImm => {
            if instruction.get("type").as_deref() != Some("Integer") {
                return Known::UNKNOWN;
            }
            match instruction
                .get("value")
                .and_then(|text| super::super::branches::bits(&text))
            {
                Some(value) => Known::exact(value),
                None => Known::UNKNOWN,
            }
        }
        CodeOp::Mov => operand("src"),
        CodeOp::And => {
            let (a, b) = (operand("lhs"), operand("rhs"));
            Known {
                ones: a.ones & b.ones,
                // A zero on either side forces a zero.
                zeros: a.zeros | b.zeros,
            }
        }
        CodeOp::Orr => {
            let (a, b) = (operand("lhs"), operand("rhs"));
            Known {
                ones: a.ones | b.ones,
                zeros: a.zeros & b.zeros,
            }
        }
        CodeOp::Eor => {
            let (a, b) = (operand("lhs"), operand("rhs"));
            // Only bits both sides know at all are known after xor.
            let both = (a.ones | a.zeros) & (b.ones | b.zeros);
            let value = a.ones ^ b.ones;
            Known {
                ones: value & both,
                zeros: !value & both,
            }
        }
        CodeOp::LslImm => shifted(operand("src"), instruction, true),
        CodeOp::LsrImm => shifted(operand("src"), instruction, false),
        CodeOp::AddImm | CodeOp::Add => {
            let (a, b) = match instruction.op {
                CodeOp::AddImm => (operand("src"), operand("imm")),
                _ => (operand("lhs"), operand("rhs")),
            };
            // Only the low run of bits both sides know, and only while no
            // carry can escape it: below the first position where either side
            // is unknown, the sum's bits are determined.
            let both_known = (a.ones | a.zeros) & (b.ones | b.zeros);
            let low_run = (!both_known).trailing_zeros();
            if low_run == 0 {
                return Known::UNKNOWN;
            }
            let mask = if low_run >= 64 {
                u64::MAX
            } else {
                (1u64 << low_run) - 1
            };
            let sum = a.ones.wrapping_add(b.ones) & mask;
            // The carry out of the known run is itself unknown territory, so
            // only bits strictly below the run's top are safe.
            let safe = mask >> 1;
            Known {
                ones: sum & safe,
                zeros: !sum & safe,
            }
        }
        _ => Known::UNKNOWN,
    }
}

/// Shift the masks; vacated bits are provably zero.
fn shifted(source: Known, instruction: &CodeInstruction, left: bool) -> Known {
    let Some(amount) = instruction
        .get("shift")
        .and_then(|text| text.parse::<u32>().ok())
    else {
        return Known::UNKNOWN;
    };
    if amount >= 64 {
        return Known::UNKNOWN; // ISA-divergent, never modeled
    }
    if source.is_unknown() && amount == 0 {
        return Known::UNKNOWN;
    }
    if left {
        let vacated = if amount == 0 { 0 } else { (1u64 << amount) - 1 };
        Known {
            ones: source.ones << amount,
            zeros: (source.zeros << amount) | vacated,
        }
    } else {
        let vacated = if amount == 0 {
            0
        } else {
            !0u64 << (64 - amount)
        };
        Known {
            ones: source.ones >> amount,
            zeros: (source.zeros >> amount) | vacated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_meet_behave() {
        let five = Known::exact(5);
        assert_eq!(five.ones, 5);
        assert_eq!(five.zeros, !5);
        assert!(five.fits_in(0xff), "5 fits in a byte");
        assert!(!Known::UNKNOWN.fits_in(0xff), "nothing is known");
        // Meeting two different constants keeps only the agreeing bits.
        let met = five.meet(Known::exact(7));
        assert_eq!(met.ones, 5 & 7);
        assert_eq!(met.zeros, !5 & !7);
    }

    #[test]
    fn left_shift_zeroes_the_vacated_bits() {
        let shifted = super::shifted(
            Known::UNKNOWN,
            &CodeInstruction::new("lsl_imm").field("shift", "8"),
            true,
        );
        assert!(
            shifted.fits_in(!0xffu64),
            "the low 8 bits are provably zero after a left shift by 8"
        );
    }

    #[test]
    fn right_shift_clears_the_high_bits() {
        let shifted = super::shifted(
            Known::UNKNOWN,
            &CodeInstruction::new("lsr_imm").field("shift", "32"),
            false,
        );
        assert!(
            shifted.fits_in(0xffff_ffff),
            "a >>32 result provably fits in 32 bits"
        );
    }

    #[test]
    fn and_with_a_mask_bounds_the_result() {
        let masked = transfer(
            &CodeInstruction::new("and")
                .field("lhs", "%v1")
                .field("rhs", "%v2"),
            |field| match field {
                "rhs" => Known::exact(0xff),
                _ => Known::UNKNOWN,
            },
        );
        assert!(masked.fits_in(0xff), "AND with 0xff fits in a byte");
    }
}
