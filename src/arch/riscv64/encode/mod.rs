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
use crate::target::shared::code::{layout_data_objects, CodeInstruction, NativeCodePlan};

// The neutral image/symbol/relocation/import containers are ISA-independent;
// reuse them rather than redeclaring a parallel set.
pub(crate) use crate::arch::aarch64::encode::{
    EncodedImage, EncodedImport, EncodedRelocation, EncodedSection, EncodedSymbol, ImportKind,
};

mod emitter;
mod operand;
mod sizing;

#[cfg(test)]
mod tests;

use emitter::Encoder;
use operand::field;
use sizing::instruction_size;

pub(crate) fn encode(plan: &NativeCodePlan) -> Result<EncodedImage, String> {
    // Partitioned data layout (bug-187): read-only constants first, then the
    // writable region; `rodata_size` marks the boundary.
    let (data, rodata_size, data_symbols) = layout_data_objects(&plan.data_objects)?;
    let mut encoder = Encoder {
        text: Vec::new(),
        data,
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

    for (name, offset) in data_symbols {
        encoder.symbols.push(EncodedSymbol {
            name,
            section: EncodedSection::Data,
            offset,
        });
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
                let name = field(instruction, "name")?;
                // A duplicate name would be last-writer-wins, silently resolving
                // every reference to the final definition (bug-127; cf. x86 bug-15).
                if let Some(first) = encoder.labels.insert(name.clone(), encoder.text.len()) {
                    return Err(format!(
                        "rv64: duplicate label '{name}' in function '{}' (first at byte {first})",
                        function.name
                    ));
                }
            } else {
                encoder
                    .text
                    .resize(encoder.text.len() + instruction_size(instruction)?, 0);
            }
        }
        encoder.text.truncate(function_start);
        // Second sub-pass: emit the bytes (label offsets are known).
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
        rodata_size,
        symbols: encoder.symbols,
        relocations: encoder.relocations,
        imports,
        entry: plan
            .entry_symbol
            .clone()
            .ok_or_else(|| "encoded image requires entry symbol".to_string())?,
        initializers: Vec::new(),
        signing_metadata: None,
        // Both are stamped by the build path after encoding: signing
        // metadata from `--sign`, and the vendor RPATH(s) from the
        // resolved native-library locators (plan-46-D §4.2/§4.3).
        rpaths: Vec::new(),
    })
}
