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

pub(crate) struct EncodedImage {
    pub(crate) text: Vec<u8>,
    pub(crate) data: Vec<u8>,
    /// Page-aligned length of the read-only constant prefix of `data` (bug-187).
    /// The linker maps `data[..rodata_size]` read-only and `data[rodata_size..]`
    /// R+W (the arena global and other mutable runtime globals). 0 = no read-only
    /// partition (the whole data segment stays writable).
    pub(crate) rodata_size: usize,
    pub(crate) symbols: Vec<EncodedSymbol>,
    pub(crate) relocations: Vec<EncodedRelocation>,
    pub(crate) imports: Vec<EncodedImport>,
    pub(crate) entry: String,
    /// Internal text symbols run, in order, after dynamic relocations and before
    /// the program entry (plan-linker.md §5.3). Materialized as ELF
    /// `DT_INIT_ARRAY` / Mach-O `S_MOD_INIT_FUNC_POINTERS`.
    pub(crate) initializers: Vec<String>,
    pub(crate) signing_metadata: Option<Vec<u8>>,
}

/// Whether an imported symbol names a function (called through a stub) or a data
/// global (addressed through the GOT). Makes linker layout deterministic without
/// scanning relocations (plan-linker.md §5.1). `Data` is produced by a
/// `tls`/app-mode consumer (and the linker tests) once one exists; the built-in
/// surface is function-only, so allow it to be otherwise-unconstructed for now.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ImportKind {
    Function,
    Data,
}

pub(crate) struct EncodedSymbol {
    pub(crate) name: String,
    pub(crate) section: EncodedSection,
    pub(crate) offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EncodedSection {
    Text,
    Data,
}

pub(crate) struct EncodedRelocation {
    pub(crate) offset: usize,
    pub(crate) target: String,
    pub(crate) kind: String,
    pub(crate) binding: String,
    pub(crate) library: Option<String>,
}

pub(crate) struct EncodedImport {
    pub(crate) library: String,
    pub(crate) symbol: String,
    /// Function (stub) vs data global (GOT-only) (plan-linker.md §5.1).
    pub(crate) kind: ImportKind,
    /// glibc symbol version this reference requires, e.g. `Some("GLIBC_2.17")`
    /// (plan-linker.md §5.2). `None` emits an unversioned reference. Ignored on
    /// Mach-O, which selects by dylib ordinal.
    pub(crate) version: Option<String>,
}

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
    })
}
