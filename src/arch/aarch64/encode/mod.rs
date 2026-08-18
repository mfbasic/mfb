use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::types::NativeCodePlan;

mod emitter;
mod operand;
mod sizing;

#[cfg(test)]
mod tests;

use emitter::Encoder;

// The sizing helper the shared driver reaches via the trait; the encoder unit
// tests also call it bare through `use super::*` (bug-341-B1).
#[cfg(test)]
use sizing::instruction_size;

// The neutral image/symbol/relocation/import containers are ISA-independent and
// live in `crate::arch::image` (bug-341-B2); re-export the ones the emitter and
// this module name so their `use super::*` resolve unchanged. `EncodedSection`
// and `ImportKind` are only named by the encoder unit tests since the shared
// `encode_plan` driver (bug-341-B1) now owns symbol/import construction.
pub(crate) use crate::arch::image::{EncodedImage, EncodedRelocation, EncodedSymbol};
#[cfg(test)]
use crate::arch::image::{EncodedSection, ImportKind};

/// Encode a plan into a linkable image. The two-pass label/emit orchestration is
/// shared across all backends (bug-341-B1); this backend supplies the AArch64
/// `Encoder` (its [`crate::arch::encode_plan::InstructionEncoder`] impl lives in
/// `emitter`) and the arch label for the duplicate-label diagnostic.
pub(crate) fn encode(plan: &NativeCodePlan) -> Result<EncodedImage, String> {
    crate::arch::encode_plan::encode_plan::<Encoder>(plan, "AArch64")
}
