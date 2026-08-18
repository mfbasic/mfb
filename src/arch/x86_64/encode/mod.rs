//! x86-64 (System V / Linux) machine-code encoder — plan-00-H Phase 1.
//!
//! Mirrors the AArch64 encoder framework (`crate::arch::aarch64::encode`) but
//! emits x86-64 machine code. The architecture-neutral container types
//! (`EncodedImage`/`EncodedSymbol`/`EncodedRelocation`/`EncodedImport`/
//! `EncodedSection`/`ImportKind`) are reused verbatim from the AArch64 encoder —
//! they describe a linkable image, not an ISA.
//!
//! The two-pass shape is identical to AArch64:
//!   1. Walk every function once to assign each text symbol an offset, using
//!      [`sizing::instruction_size`] — which MUST return exactly the byte count
//!      [`emitter::Encoder::emit_instruction`] produces for the same instruction.
//!   2. Re-walk per function: record `label` offsets, then emit bytes, then
//!      [`emitter::Encoder::patch_labels`] resolves intra-function branch
//!      displacements (rel32). Inter-function / data references are emitted as
//!      relocations.
//!
//! Phase 1 implements the full scalar-integer core (the instruction families the
//! prompt lists). Float / `v128` ops return a clear `Err` — they are Phase 2/3.
//!
//! ## `adrp` / `add_pageoff` → RIP-relative `lea`
//!
//! AArch64 forms a data address as an `adrp; add :lo12:` page pair (two
//! relocations). x86-64 references memory RIP-relative in a single instruction,
//! so this encoder collapses the pair: `adrp {dst,symbol}` emits
//! `lea dst, [rip+disp32]` with a single `data_pc32` relocation against the
//! disp32 field for an internal data symbol. For an **imported** symbol the same
//! form is rewritten to `mov dst, [rip+disp32]` (REX.W 0x8B) with a `got_pc32`
//! relocation so the GOT slot is dereferenced once (`lea` would leave the GOT
//! slot's address in `dst`, one indirection short — bug-192). The following
//! `add_pageoff {dst,…}` emits **zero bytes** (the full address is already in
//! `dst`). See [`emitter::Encoder::emit_symbol_ref`] and the opcode rewrite in
//! [`emitter::Encoder::emit_instruction`].

use std::collections::HashMap;

use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::types::NativeCodePlan;

// The neutral image/symbol/relocation/import containers are ISA-independent
// (`crate::arch::image`, bug-341-B2). Re-export the ones the emitter and this
// module name; `EncodedSection`/`ImportKind` are only named by the unit tests
// now that the shared `encode_plan` driver (bug-341-B1) owns symbol/import
// construction.
#[cfg(test)]
use crate::arch::image::EncodedSection;
pub(crate) use crate::arch::image::{EncodedImage, EncodedRelocation, EncodedSymbol};

mod emitter;
mod operand;
mod sizing;

#[cfg(test)]
mod tests;

use emitter::Encoder;

/// Encode a plan into a linkable image via the shared two-pass driver
/// (bug-341-B1); this backend supplies the x86-64 `Encoder` (its
/// [`crate::arch::encode_plan::InstructionEncoder`] impl lives in `emitter`).
pub(crate) fn encode(plan: &NativeCodePlan) -> Result<EncodedImage, String> {
    crate::arch::encode_plan::encode_plan::<Encoder>(plan, "x86_64")
}
