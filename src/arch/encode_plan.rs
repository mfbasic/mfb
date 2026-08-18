//! The shared two-pass plan-to-image encoding driver (bug-341-B1).
//!
//! Every backend encoder ran a byte-identical `encode()`: lay out the data
//! segment, register the data and function symbols, place labels by reserving
//! each instruction's exact size in a first sub-pass, then emit the bytes in a
//! second sub-pass and resolve the label patches. The three copies had already
//! drifted into five divergent explanations of the same two sub-passes and
//! differed only in the arch-name in the duplicate-label panic.
//!
//! This module owns that orchestration once. Each backend supplies an
//! [`InstructionEncoder`] — the small ISA-specific surface the driver calls
//! (construct, size, emit, patch) — and its `encode()` becomes a one-line
//! delegation to [`encode_plan`] with its arch label.

use std::collections::HashMap;

use crate::arch::image::{
    EncodedImage, EncodedImport, EncodedRelocation, EncodedSection, EncodedSymbol, ImportKind,
};
use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::layout_data_objects;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::types::NativeCodePlan;

/// The per-ISA surface [`encode_plan`] drives. Each backend's `Encoder`
/// implements it; the byte production and relocation/label bookkeeping stay in
/// the backend, the two-pass orchestration is shared.
pub(crate) trait InstructionEncoder: Sized {
    /// Construct a fresh encoder seeded with the laid-out data segment and the
    /// plan's import table (symbol name → library).
    fn new(data: Vec<u8>, imports: HashMap<String, String>) -> Self;

    /// The exact encoded byte length of one instruction — the value the first
    /// sub-pass reserves so label offsets are known before any bytes are emitted.
    fn instruction_size(instruction: &CodeInstruction) -> Result<usize, String>;

    /// The `name` field of a `Label` pseudo-instruction.
    fn label_name(instruction: &CodeInstruction) -> Result<String, String>;

    /// Emit one instruction's bytes into `text`, recording any relocation or
    /// label patch as a side effect. (Named `emit_one` rather than
    /// `emit_instruction` so a backend's inherent `emit_instruction` — which the
    /// encoder unit tests call directly — is delegated to, not shadowed.)
    fn emit_one(&mut self, instruction: &CodeInstruction) -> Result<(), String>;

    /// Resolve the recorded label patches now that every label offset is known.
    fn resolve_patches(&mut self) -> Result<(), String>;

    /// Current length of the text section (a label's byte offset / the running
    /// reservation cursor).
    fn text_len(&self) -> usize;

    /// Grow the text section to `len` bytes, zero-filling — the first sub-pass's
    /// size reservation.
    fn reserve_text(&mut self, len: usize);

    /// Drop the reserved bytes back to `len` before the emitting sub-pass.
    fn truncate_text(&mut self, len: usize);

    /// Register an encoded symbol (a data global or a function entry).
    fn push_symbol(&mut self, symbol: EncodedSymbol);

    /// Record `name → offset`, returning the previous offset if the label was
    /// already defined in this function (a duplicate the caller rejects).
    fn insert_label(&mut self, name: String, offset: usize) -> Option<usize>;

    /// Clear the per-function label table before each function.
    fn clear_labels(&mut self);

    /// Clear the per-function label-patch list after each function is resolved.
    fn clear_patches(&mut self);

    /// Decompose into the encoded-image parts: `(text, data, symbols,
    /// relocations)`.
    fn into_parts(self) -> (Vec<u8>, Vec<u8>, Vec<EncodedSymbol>, Vec<EncodedRelocation>);
}

/// Encode a native code plan into a linkable [`EncodedImage`]. `arch_name` labels
/// the duplicate-label diagnostic (`"AArch64"` / `"x86_64"` / `"rv64"`).
pub(crate) fn encode_plan<E: InstructionEncoder>(
    plan: &NativeCodePlan,
    arch_name: &str,
) -> Result<EncodedImage, String> {
    // Partitioned data layout (bug-187): read-only constants first, then the
    // writable region; `rodata_size` marks the boundary and every Data symbol's
    // offset comes from the same pass.
    let (data, rodata_size, data_symbols) = layout_data_objects(&plan.data_objects)?;
    let imports = plan
        .imports
        .iter()
        .map(|import| (import.symbol.clone(), import.library.clone()))
        .collect();
    let mut encoder = E::new(data, imports);

    for (name, offset) in data_symbols {
        encoder.push_symbol(EncodedSymbol {
            name,
            section: EncodedSection::Data,
            offset,
        });
    }

    let mut text_offset = 0;
    for function in &plan.functions {
        encoder.push_symbol(EncodedSymbol {
            name: function.symbol.clone(),
            section: EncodedSection::Text,
            offset: text_offset,
        });
        for instruction in &function.instructions {
            text_offset += E::instruction_size(instruction)?;
        }
    }

    for function in &plan.functions {
        encoder.clear_labels();
        let function_start = encoder.text_len();
        // First sub-pass: place each label at its byte offset by reserving each
        // non-label instruction's exact size.
        for instruction in &function.instructions {
            if instruction.op == CodeOp::Label {
                let name = E::label_name(instruction)?;
                // A duplicate name would be last-writer-wins, silently resolving
                // every reference to the final definition (bug-127; cf. bug-15).
                let here = encoder.text_len();
                if let Some(first) = encoder.insert_label(name.clone(), here) {
                    return Err(format!(
                        "{arch_name}: duplicate label '{name}' in function '{}' (first at byte {first})",
                        function.name
                    ));
                }
            } else {
                let reserved = encoder.text_len() + E::instruction_size(instruction)?;
                encoder.reserve_text(reserved);
            }
        }
        encoder.truncate_text(function_start);
        // Second sub-pass: emit the bytes (label offsets are now known).
        for instruction in &function.instructions {
            encoder.emit_one(instruction)?;
        }
        encoder.resolve_patches()?;
        encoder.clear_patches();
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

    let (text, data, symbols, relocations) = encoder.into_parts();
    Ok(EncodedImage {
        text,
        data,
        rodata_size,
        symbols,
        relocations,
        imports,
        entry: plan
            .entry_symbol
            .clone()
            .ok_or_else(|| "encoded image requires entry symbol".to_string())?,
        initializers: Vec::new(),
        signing_metadata: None,
        // Both are stamped by the build path after encoding: signing metadata
        // from `--sign`, and the vendor RPATH(s) from the resolved
        // native-library locators (plan-46-D §4.2/§4.3).
        rpaths: Vec::new(),
    })
}
