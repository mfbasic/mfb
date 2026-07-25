use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::target::shared::code::{layout_data_objects, CodeInstruction, NativeCodePlan};

mod emitter;
mod operand;
mod sizing;

#[cfg(test)]
mod tests;

use emitter::Encoder;
use operand::field;
use sizing::instruction_size;

// The neutral image/symbol/relocation/import containers are ISA-independent and
// live in `crate::arch::image` (bug-341-B2); re-export them so this module's
// `encode()` and the `use super::*` in the emitter/tests resolve them unchanged.
pub(crate) use crate::arch::image::{
    EncodedImage, EncodedImport, EncodedRelocation, EncodedSection, EncodedSymbol, ImportKind,
};

pub(crate) fn encode(plan: &NativeCodePlan) -> Result<EncodedImage, String> {
    // Partitioned data layout (bug-187): read-only constants first, then the
    // writable region; `rodata_size` marks the boundary and every Data symbol's
    // offset comes from the same pass.
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
        for instruction in &function.instructions {
            if instruction.op == CodeOp::Label {
                let name = field(instruction, "name")?;
                // A duplicate name would be last-writer-wins, silently resolving
                // every reference to the final definition (bug-127; cf. x86 bug-15).
                if let Some(first) = encoder.labels.insert(name.clone(), encoder.text.len()) {
                    return Err(format!(
                        "AArch64: duplicate label '{name}' in function '{}' (first at byte {first})",
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
            // The built-in surface is function-only and unversioned; a versioned
            // or data import is supplied by a `tls`/app-mode consumer once one
            // exists (plan-linker.md §3.1). Default accordingly.
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
