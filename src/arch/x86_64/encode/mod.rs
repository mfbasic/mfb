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
//! `lea dst, [rip+disp32]` with a single `data_pc32` (or `got_pc32` for an
//! imported symbol) relocation against the disp32 field, and the following
//! `add_pageoff {dst,…}` emits **zero bytes** (the full address is already in
//! `dst`). See [`emitter::Encoder::emit_symbol_ref`].

use std::collections::HashMap;

use crate::arch::aarch64::ops::CodeOp;
use crate::target::shared::code::{CodeInstruction, NativeCodePlan};

// The neutral image/symbol/relocation/import containers are ISA-independent;
// reuse them rather than redeclaring a parallel set.
pub(crate) use crate::arch::aarch64::encode::{
    EncodedImage, EncodedImport, EncodedRelocation, EncodedSection, EncodedSymbol, ImportKind,
};

mod data;
mod emitter;
mod operand;
mod sizing;

#[cfg(test)]
mod tests;

use data::{align, encode_data};
use emitter::Encoder;
use operand::field;
use sizing::instruction_size;

pub(crate) fn encode(plan: &NativeCodePlan) -> Result<EncodedImage, String> {
    let mut encoder = Encoder {
        text: Vec::new(),
        data: encode_data(plan)?,
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: plan
            .imports
            .iter()
            .map(|import| (import.symbol.clone(), import.library.clone()))
            .collect(),
        labels: HashMap::new(),
        patches: Vec::new(),
    };

    let mut data_offset = 0;
    for object in &plan.data_objects {
        data_offset = align(data_offset, object.align);
        encoder.symbols.push(EncodedSymbol {
            name: object.symbol.clone(),
            section: EncodedSection::Data,
            offset: data_offset,
        });
        data_offset += object.size;
    }

    let mut text_offset = 0;
    for function in &plan.functions {
        encoder.symbols.push(EncodedSymbol {
            name: function.symbol.clone(),
            section: EncodedSection::Text,
            offset: text_offset,
        });
        for instruction in &function.instructions {
            text_offset += instruction_size(instruction)?;
        }
    }

    for function in &plan.functions {
        encoder.labels.clear();
        let function_start = encoder.text.len();
        // First sub-pass: place each label at its byte offset by reserving each
        // non-label instruction's exact size.
        for instruction in &function.instructions {
            if instruction.op == CodeOp::Label {
                encoder
                    .labels
                    .insert(field(instruction, "name")?, encoder.text.len());
            } else {
                encoder
                    .text
                    .resize(encoder.text.len() + instruction_size(instruction)?, 0);
            }
        }
        encoder.text.truncate(function_start);
        // Second sub-pass: actually emit the bytes (label offsets are known).
        for instruction in &function.instructions {
            encoder.emit_instruction(instruction)?;
        }
        encoder.patch_labels()?;
        encoder.patches.clear();
    }

    let imports = plan
        .imports
        .iter()
        .map(|import| EncodedImport {
            library: import.library.clone(),
            symbol: import.symbol.clone(),
            kind: ImportKind::Function,
            version: None,
        })
        .collect();

    Ok(EncodedImage {
        text: encoder.text,
        data: encoder.data,
        symbols: encoder.symbols,
        relocations: encoder.relocations,
        imports,
        entry: plan
            .entry_symbol
            .clone()
            .ok_or_else(|| "encoded image requires entry symbol".to_string())?,
        initializers: Vec::new(),
        signing_metadata: None,
    })
}
