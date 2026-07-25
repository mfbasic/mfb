//! RISC-V 64 (RVA20 / RV64GC, Linux lp64d) machine-code encoder — plan-99.
//!
//! Mirrors the AArch64 encoder framework (`crate::arch::aarch64::encode`) but
//! emits little-endian RV64GC machine code. The architecture-neutral container
//! types (`EncodedImage`/`EncodedSymbol`/…) are reused verbatim from the AArch64
//! encoder — they describe a linkable image, not an ISA.
//!
//! The two-pass shape is identical to the other backends:
//!   1. Walk every function once to assign each text symbol an offset, using
//!      [`sizing::instruction_size`] — which MUST return exactly the byte count
//!      [`emitter::Encoder::emit_instruction`] produces for the same instruction.
//!   2. Re-walk per function: record `label` offsets, then emit bytes, then
//!      [`emitter::Encoder::patch_labels`] resolves intra-function branch
//!      displacements. Inter-function / data references are emitted as
//!      relocations for the linker.
//!
//! Because RISC-V has fixed 32-bit instructions but tighter branch reach than
//! AArch64 (a conditional branch reaches only ±4 KiB, versus AArch64's ±1 MiB),
//! the conditional-branch op [`CodeOp::RvBr`] is always emitted in its 8-byte
//! long form (`b<inverse> rs1, rs2, +8; jal zero, target`) so its size is
//! deterministic and it reaches ±1 MiB — no branch-relaxation pass is needed.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::target::shared::code::{CodeInstruction, NativeCodePlan};

// The neutral image/symbol/relocation/import containers are ISA-independent
// (`crate::arch::image`, bug-341-B2). Re-export the ones the emitter and this
// module name; `EncodedSection`/`ImportKind` are only named by the unit tests
// now that the shared `encode_plan` driver (bug-341-B1) owns symbol/import
// construction.
pub(crate) use crate::arch::image::{EncodedImage, EncodedRelocation, EncodedSymbol};

mod emitter;
mod operand;
mod sizing;

#[cfg(test)]
mod tests;

use emitter::Encoder;

/// Encode a plan into a linkable image via the shared two-pass driver
/// (bug-341-B1); this backend supplies the RV64 `Encoder` (its
/// [`crate::arch::encode_plan::InstructionEncoder`] impl lives in `emitter`).
pub(crate) fn encode(plan: &NativeCodePlan) -> Result<EncodedImage, String> {
    crate::arch::encode_plan::encode_plan::<Encoder>(plan, "rv64")
}
